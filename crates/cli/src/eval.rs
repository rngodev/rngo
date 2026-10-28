mod status;

use console::style;
use rngo::{Dialect, RunLogWriter, SignalOutcome, SqliteRunLog, StopHandle, spec};
use status::StatusWriter;
use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::{fmt, fs};
use uuid::Uuid;

/// Options for a single `rngo eval`.
#[derive(Default)]
pub struct EvalOptions {
    /// Print events to stdout instead of sending them to channels.
    pub stdout: bool,
    /// Only check that the simulation builds.
    pub dry_run: bool,
    /// Cap on the number of inputs produced.
    pub limit: Option<std::num::NonZeroU64>,
    /// Wait for each input's timestamp before sending it instead of sending right away.
    pub realtime: bool,
    /// Stops the eval early; it then finishes and audits what was logged.
    pub stop: StopHandle,
}

/// Runs a simulation from a spec file, a `.rngo` directory, or a directory containing one (default `./.rngo`); returns whether it completed with every signal passing.
pub fn eval(path: Option<&Path>, options: EvalOptions) -> Result<bool, Box<dyn Error>> {
    let EvalOptions {
        stdout,
        dry_run,
        limit,
        realtime,
        stop,
    } = options;

    let Source {
        spec,
        rngo_dir,
        base,
    } = resolve_source(path)?;

    let _ = dotenvy::from_path(base.join(".env"));

    let dialect = Dialect::primitive();

    let rewritten = stdout.then(|| to_stdout_channels(&spec));
    let effective = rewritten.as_ref().unwrap_or(&spec);

    let mut simulation_builder = dialect
        .parse_simulation(spec.clone())
        .map_err(join_errors)?;

    let mut proxy_builder = dialect
        .parse_proxy(effective.clone())
        .map_err(join_errors)?;

    let audit_builder = dialect.parse_audit(spec.clone()).map_err(join_errors)?;

    if let Some(limit) = limit {
        simulation_builder = simulation_builder.limit(limit.get());
    }

    if dry_run {
        simulation_builder
            .run_log_reader(rngo::SimpleEventRunLog::new())
            .build()
            .map_err(join_errors)?;
        return Ok(true);
    }

    let eval_dir = prepare_eval_dir(&rngo_dir, &spec, rewritten.as_ref())?;

    let sqlite_run_log = SqliteRunLog::new(eval_dir.clone())?;
    let reader = sqlite_run_log.clone();
    let writer = StatusWriter::new(sqlite_run_log.clone(), &spec);

    let flush_failure = Rc::new(RefCell::new(None));

    if realtime {
        let status = writer.clone();
        let failure = flush_failure.clone();
        proxy_builder.set_realtime(true).set_on_wait(move |until| {
            if let Err(error) = status.flush() {
                failure.borrow_mut().get_or_insert(error);
            }
            status.wait(until);
        });
    }

    let mut proxy = proxy_builder
        .run_log_writer(writer.clone())
        .stop_handle(stop.clone())
        .build()
        .map_err(join_errors)?;

    let simulation = simulation_builder
        .run_log_reader(reader.clone())
        .build()
        .map_err(join_errors)?;

    for item in simulation {
        match item {
            Ok(input) => proxy.send(&input)?,
            Err(skipped) => writer.push_metadata(skipped.into())?,
        }
        if let Some(error) = flush_failure.borrow_mut().take() {
            return Err(error.into());
        }
        if stop.is_stopped() {
            break;
        }
    }

    proxy.finish()?;
    writer.finish();

    let audit = audit_builder
        .run_log(sqlite_run_log.clone())
        .build()
        .map_err(join_errors)?;

    let audit_report = audit.run()?;
    sqlite_run_log.commit()?;

    if !audit_report.outcomes.is_empty() {
        println!();
        println!("{}", style("Audit").bold());

        for (key, outcome) in &audit_report.outcomes {
            match outcome {
                SignalOutcome::Error { error } => {
                    println!("{key}: error - {error}");
                }
                SignalOutcome::Success { value, eval } => match eval {
                    Some(eval) => {
                        if eval.passed {
                            println!("{key}: {value} (passed)");
                        } else {
                            let expectation = eval.expectation.clone();
                            println!("{key}: {value} (failed - expected '{expectation}')");
                        }
                    }
                    None => println!("{key}: {value}"),
                },
            }
        }

        if audit_report.eval_count() > 0 {
            println!("{} passed", audit_report.pass_count());
            println!("{} failed", audit_report.fail_count());
        }

        if audit_report.error_count() > 0 {
            println!("{} errored", audit_report.error_count());
        }
    }

    Ok(audit_report.passed() && !stop.is_stopped())
}

/// Reads a YAML file that must hold a mapping, naming the file in any error.
fn read_yaml(path: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_yaml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if !value.is_object() {
        return Err(format!("{}: expected a YAML mapping", path.display()).into());
    }
    Ok(value)
}

fn load_spec_file(path: &Path) -> Result<spec::Spec, Box<dyn Error>> {
    let value: serde_json::Value = read_yaml(path)?;
    Ok(spec::from_value(value).map_err(join_errors)?)
}

/// Where an eval's spec comes from, and the directory its `.env` and `evals/` belong to.
struct Source {
    spec: spec::Spec,
    rngo_dir: PathBuf,
    base: PathBuf,
}

/// Resolves the path argument: a file is the whole spec, a directory named `.rngo` is built from,
/// any other directory is searched for a `.rngo` inside, and no path means `./.rngo`.
fn resolve_source(path: Option<&Path>) -> Result<Source, Box<dyn Error>> {
    let Some(path) = path else {
        let rngo_dir = PathBuf::from(".rngo");
        return Ok(Source {
            spec: load_spec(&rngo_dir)?,
            rngo_dir,
            base: PathBuf::from("."),
        });
    };
    if path.is_file() {
        return Ok(Source {
            spec: load_spec_file(path)?,
            rngo_dir: PathBuf::from(".rngo"),
            base: PathBuf::from("."),
        });
    }
    if !path.is_dir() {
        return Err(format!("{}: no such file or directory", path.display()).into());
    }
    let rngo_dir = if path.file_name().is_some_and(|name| name == ".rngo") {
        path.to_path_buf()
    } else {
        let inner = path.join(".rngo");
        if !inner.is_dir() {
            return Err(format!("{}: no .rngo directory found", path.display()).into());
        }
        inner
    };
    let base = match rngo_dir.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    Ok(Source {
        spec: load_spec(&rngo_dir)?,
        rngo_dir,
        base,
    })
}

/// Builds a spec from `spec.yml` in `rngo_dir`, merged with each section's per-file `*.yml`, keyed by file stem.
fn load_spec(rngo_dir: &Path) -> Result<spec::Spec, Box<dyn Error>> {
    let spec_path = rngo_dir.join("spec.yml");

    let mut spec: serde_json::Value = if spec_path.exists() {
        read_yaml(&spec_path)?
    } else {
        serde_json::json!({ "seed": 1, "effects": {} })
    };

    for section in ["effects", "channels", "generators", "signals"] {
        merge_section(&mut spec, rngo_dir, section)?;
    }

    Ok(spec::from_value(spec).map_err(join_errors)?)
}

fn merge_section(
    spec: &mut serde_json::Value,
    rngo_dir: &Path,
    section: &str,
) -> Result<(), Box<dyn Error>> {
    if !spec[section].is_object() {
        spec[section] = serde_json::json!({});
    }

    let section_dir = rngo_dir.join(section);
    if !section_dir.is_dir() {
        return Ok(());
    }

    let mut paths: Vec<_> = fs::read_dir(&section_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yml"))
        .collect();
    paths.sort();

    for path in paths {
        let key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("invalid filename: {}", path.display()))?
            .to_string();
        spec[section][key] = read_yaml(&path)?;
    }
    Ok(())
}

/// Replaces every channel with a `stdout` one that keeps only its `format`.
fn to_stdout_channels(spec: &spec::Spec) -> spec::Spec {
    let mut spec = spec.clone();
    for channel in spec.channels.values_mut() {
        let format = channel.fields.get("format").cloned();
        channel.ctype = Some("stdout".into());
        channel.fields.clear();
        channel
            .fields
            .extend(format.map(|f| ("format".to_string(), f)));
    }
    spec
}

/// Creates the eval directory with `spec.json`, plus `spec.rewritten.json` when the spec that ran
/// differs from the one written.
fn prepare_eval_dir(
    rngo_dir: &Path,
    spec: &spec::Spec,
    rewritten: Option<&spec::Spec>,
) -> Result<PathBuf, Box<dyn Error>> {
    let eval_dir = new_eval_dir(rngo_dir)?;
    fs::create_dir_all(&eval_dir)?;
    fs::write(
        eval_dir.join("spec.json"),
        serde_json::to_string_pretty(spec)?,
    )?;
    if let Some(rewritten) = rewritten {
        fs::write(
            eval_dir.join("spec.rewritten.json"),
            serde_json::to_string_pretty(rewritten)?,
        )?;
    }
    update_last_symlink(rngo_dir, &eval_dir)?;
    Ok(eval_dir)
}

fn new_eval_dir(rngo_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let evals_dir = rngo_dir.join("evals");
    fs::create_dir_all(&evals_dir)?;
    let id = Uuid::now_v7();
    Ok(evals_dir.join(id.to_string()))
}

fn update_last_symlink(rngo_dir: &Path, eval_dir: &Path) -> Result<(), Box<dyn Error>> {
    let link = rngo_dir.join("evals/last");
    if link.exists() || link.is_symlink() {
        fs::remove_file(&link)?;
    }
    let target = eval_dir.strip_prefix(rngo_dir.join("evals"))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &link)?;
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(target, &link)?;
    Ok(())
}

fn join_errors<E: fmt::Display>(errors: Vec<E>) -> String {
    errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
