mod status;

use console::style;
use rngo::{Dialect, RunLogWriter, SignalOutcome, SqliteRunLog, StopHandle, spec};
use status::StatusWriter;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::{fmt, fs};
use uuid::Uuid;

/// Options for a single `rngo eval`.
#[derive(Default)]
pub struct EvalOptions<'a> {
    /// Print events to stdout instead of sending them to channels.
    pub stdout: bool,
    /// Load this spec file instead of the `.rngo` directory.
    pub spec_path: Option<&'a Path>,
    /// Only check that the simulation builds.
    pub dry_run: bool,
    /// Cap on the number of inputs produced.
    pub limit: Option<std::num::NonZeroU64>,
    /// Wait for each input's timestamp before sending it instead of sending right away.
    pub realtime: bool,
    /// Stops the eval early; it then finishes and audits what was logged.
    pub stop: StopHandle,
}

/// Runs a simulation from the `.rngo` directory under `base`; returns whether it completed with every signal passing.
pub fn eval(base: &Path, options: EvalOptions) -> Result<bool, Box<dyn Error>> {
    let EvalOptions {
        stdout,
        spec_path,
        dry_run,
        limit,
        realtime,
        stop,
    } = options;

    let _ = dotenvy::from_path(base.join(".env"));

    let spec = match spec_path {
        Some(path) => load_spec_file(path)?,
        None => load_spec(base)?,
    };

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

    let eval_dir = prepare_eval_dir(base, &spec, rewritten.as_ref())?;

    let sqlite_run_log = SqliteRunLog::new(eval_dir.clone());
    let reader = sqlite_run_log.clone();
    let writer = StatusWriter::new(sqlite_run_log.clone(), &spec);

    if realtime {
        let status = writer.clone();
        proxy_builder.set_realtime(true).set_on_wait(move |until| {
            status.flush();
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
            Err(skipped) => writer.push_metadata(skipped.into()),
        }
        if stop.is_stopped() {
            break;
        }
    }

    proxy.finish();
    writer.finish();

    let audit = audit_builder
        .run_log(sqlite_run_log)
        .build()
        .map_err(join_errors)?;

    let audit_report = audit.run();

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

fn load_spec_file(path: &Path) -> Result<spec::Spec, Box<dyn Error>> {
    let value: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    Ok(spec::from_value(value).map_err(join_errors)?)
}

fn load_spec(base: &Path) -> Result<spec::Spec, Box<dyn Error>> {
    let spec_path = base.join(".rngo/spec.yml");

    let mut spec: serde_json::Value = if spec_path.exists() {
        serde_yaml::from_str(&fs::read_to_string(&spec_path)?)?
    } else {
        serde_json::json!({ "seed": 1, "effects": {} })
    };

    if !spec["effects"].is_object() {
        spec["effects"] = serde_json::json!({});
    }

    let effects_dir = base.join(".rngo/effects");
    if effects_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&effects_dir)?
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
            let effect: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["effects"][key] = effect;
        }
    }

    if !spec["channels"].is_object() {
        spec["channels"] = serde_json::json!({});
    }

    let channels_dir = base.join(".rngo/channels");
    if channels_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&channels_dir)?
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
            let channel: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["channels"][key] = channel;
        }
    }

    if !spec["generators"].is_object() {
        spec["generators"] = serde_json::json!({});
    }

    let generators_dir = base.join(".rngo/generators");
    if generators_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&generators_dir)?
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
            let generator: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["generators"][key] = generator;
        }
    }

    if !spec["signals"].is_object() {
        spec["signals"] = serde_json::json!({});
    }

    let signals_dir = base.join(".rngo/signals");
    if signals_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&signals_dir)?
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
            let signal: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["signals"][key] = signal;
        }
    }

    Ok(spec::from_value(spec).map_err(join_errors)?)
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
    base: &Path,
    spec: &spec::Spec,
    rewritten: Option<&spec::Spec>,
) -> Result<PathBuf, Box<dyn Error>> {
    let eval_dir = new_eval_dir(base)?;
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
    update_last_symlink(base, &eval_dir)?;
    Ok(eval_dir)
}

fn new_eval_dir(base: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let evals_dir = base.join(".rngo/evals");
    fs::create_dir_all(&evals_dir)?;
    let id = Uuid::now_v7();
    Ok(evals_dir.join(id.to_string()))
}

fn update_last_symlink(base: &Path, eval_dir: &Path) -> Result<(), Box<dyn Error>> {
    let link = base.join(".rngo/evals/last");
    if link.exists() || link.is_symlink() {
        fs::remove_file(&link)?;
    }
    let target = eval_dir.strip_prefix(base.join(".rngo/evals"))?;
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
