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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write_yaml(path: impl AsRef<Path>, value: &serde_json::Value) {
        fs::write(path, serde_yaml::to_string(value).unwrap()).unwrap();
    }

    fn signal_outcome(base: &Path, key: &str) -> (serde_json::Value, bool) {
        let connection =
            rusqlite::Connection::open(base.join(".rngo/evals/last/log.sqlite")).unwrap();
        let mut statement = connection
            .prepare("SELECT data FROM metadata WHERE type = 'signal'")
            .unwrap();
        let outcome = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|data| serde_json::from_str::<serde_json::Value>(&data.unwrap()).unwrap())
            .find(|outcome| outcome["key"] == key)
            .unwrap_or_else(|| panic!("no signal metadata found for key {key}"));

        let passed = outcome["eval"]["passed"].as_bool().unwrap_or(false);
        (outcome["value"].clone(), passed)
    }

    fn write_stdout_fixture(base: &Path, marker: &Path) {
        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({ "seed": 1, "start": "2024-01-01", "end": "2024-01-04" }),
        );
        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": { "type": "constant", "value": "x" }
            }),
        );
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "type": "exec",
                "format": { "type": "template", "template": format!("touch {}", marker.display()) }
            }),
        );
        write_yaml(
            base.join(".rngo/channels/tail.yml"),
            &json!({ "type": "stream", "command": format!("touch {}; cat", marker.display()) }),
        );
    }

    #[test]
    fn stdout_prints_instead_of_running_channels_and_records_the_rewritten_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let marker = base.join("ran");
        write_stdout_fixture(base, &marker);

        eval(
            base,
            EvalOptions {
                stdout: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(
            !marker.exists(),
            "no channel should have run under --stdout"
        );

        let read = |name: &str| -> serde_json::Value {
            serde_json::from_str(
                &fs::read_to_string(base.join(".rngo/evals/last").join(name)).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(read("spec.json")["channels"]["logger"]["type"], "exec");
        let rewritten = read("spec.rewritten.json");
        assert_eq!(rewritten["channels"]["logger"]["type"], "stdout");
        assert_eq!(
            rewritten["channels"]["logger"]["format"]["type"],
            "template"
        );
        assert_eq!(rewritten["channels"]["tail"], json!({ "type": "stdout" }));
    }

    #[test]
    fn no_rewritten_spec_is_recorded_without_stdout() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        write_stdout_fixture(base, &base.join("ran"));

        eval(base, EvalOptions::default()).unwrap();

        assert!(base.join(".rngo/evals/last/spec.json").exists());
        assert!(!base.join(".rngo/evals/last/spec.rewritten.json").exists());
    }

    #[test]
    fn exec_channel_runs_command_per_event() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("exec_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "echo {{data.id}} >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": { "type": "template", "template": command },
                "type": "exec"
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "exec command should have run once per event"
        );
    }

    #[test]
    fn exec_channel_runs_string_data_without_a_format() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("exec_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        let command = "echo hi >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": { "type": "constant", "value": command }
            }),
        );

        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({ "type": "exec" }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(content.lines().all(|line| line == "hi"));
        assert!(content.lines().count() > 0);
    }

    #[test]
    fn exec_channel_records_output_when_command_fails() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/signals/has-failure-output.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM outputs WHERE data LIKE 'command exited with%'",
                "expect": "result >= 1"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": { "type": "template", "template": "exit 1" },
                "type": "exec"
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let (_, passed) = signal_outcome(base, "has-failure-output");
        assert!(passed);
    }

    #[test]
    fn stream_channel_does_not_drop_trailing_output() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        // Every event fed to `cat` is echoed straight back out over stdout, so the number of
        // outputs recorded should match the number of effects exactly - including the one
        // produced by the very last event, which arrives only after the subprocess is closed.
        write_yaml(
            base.join(".rngo/signals/matches-effect-count.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT (SELECT COUNT(*) FROM inputs) - (SELECT COUNT(*) FROM outputs)",
                "expect": "result == 0"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "type": "stream", "command": "cat"
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let (value, passed) = signal_outcome(base, "matches-effect-count");
        assert!(
            passed,
            "expected output count to match effect count, got {value}"
        );
    }

    #[test]
    fn stream_channel_pipes_events_to_subprocess() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("stream_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "type": "stream", "command": command
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "stream subprocess should have received events"
        );
    }

    #[test]
    fn template_format_is_written_to_a_stream_channel() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("template_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": { "tag": { "type": "constant", "value": "<a&b>" } }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": { "type": "template", "template": "{{effect.key}} {{data.tag}} {{json data}}" },
                "type": "stream", "command": command
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        let first = content
            .lines()
            .next()
            .expect("stream should receive events");
        assert_eq!(first, r#"ping <a&b> {"tag":"<a&b>"}"#);
    }

    #[test]
    fn sql_format_uses_table_from_effect_metadata() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("sql_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-02"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "db",
                "metadata": { "table": "accounts" },
                "trigger": "hz(1, hour)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/db.yml"),
            &json!({
                "format": { "type": "sql" },
                "type": "stream", "command": command
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "expected at least one formatted line"
        );
        for line in content.lines() {
            assert!(
                line.starts_with("INSERT INTO accounts ("),
                "expected sql format using metadata table, got: {line}"
            );
        }
    }

    #[test]
    fn spec_flag_uses_given_file_instead_of_rngo_dir() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("spec_flag_output.txt");

        // A broken `.rngo/spec.yml` proves it is never read when `--spec` is used.
        fs::create_dir_all(base.join(".rngo")).unwrap();
        fs::write(base.join(".rngo/spec.yml"), "not: [valid").unwrap();

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        let spec_path = base.join("external_spec.yml");
        write_yaml(
            &spec_path,
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04",
                "channels": {
                    "logger": {
                        "format": {},
                        "type": "stream", "command": command
                    }
                },
                "effects": {
                    "ping": {
                        "channel": "logger",
                        "trigger": "hz(1, day)",
                        "generator": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                            }
                        }
                    }
                }
            }),
        );

        eval(
            base,
            EvalOptions {
                spec_path: Some(&spec_path),
                ..Default::default()
            },
        )
        .unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "eval should use the spec file passed via --spec"
        );
    }

    #[test]
    fn generators_dir_yml_files_are_referenceable_by_type_name() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("generators_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/generators")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-02"
            }),
        );

        write_yaml(
            base.join(".rngo/generators/title.yml"),
            &json!({
                "generator": {
                    "type": "select",
                    "options": [
                        { "generator": { "type": "constant", "value": "Mr." } },
                        { "generator": { "type": "constant", "value": "Mrs." } },
                        { "generator": { "type": "constant", "value": "Dr." } }
                    ]
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, hour)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "title" }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": {},
                "type": "stream", "command": command
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        let lines: Vec<_> = content.lines().collect();
        assert!(!lines.is_empty(), "expected at least one emitted event");
        for line in lines {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let title = value["title"].as_str().unwrap();
            assert!(
                ["Mr.", "Mrs.", "Dr."].contains(&title),
                "unexpected title {title:?}"
            );
        }
    }

    #[test]
    fn signals_are_evaluated_and_written_to_eval_dir() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04",
                "signals": {
                    "hasEvents": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM inputs",
                        "expect": "result >= 1"
                    },
                    "tooMany": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM inputs",
                        "expect": "result > 1000"
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let (has_events_value, has_events_passed) = signal_outcome(base, "hasEvents");
        assert!(has_events_passed);
        assert!(has_events_value.as_i64().unwrap() >= 1);

        let (_, too_many_passed) = signal_outcome(base, "tooMany");
        assert!(!too_many_passed);
    }

    #[test]
    fn signals_dir_yml_files_are_merged_into_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/signals/has-events.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM inputs",
                "expect": "result >= 1"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let (_, passed) = signal_outcome(base, "has-events");
        assert!(passed);
    }

    #[test]
    fn run_returns_false_when_a_signal_fails() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04",
                "signals": {
                    "tooMany": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM inputs",
                        "expect": "result > 1000"
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let passed = eval(base, EvalOptions::default()).unwrap();
        assert!(!passed, "eval should fail when a signal's expect fails");
    }

    #[test]
    fn run_returns_false_when_a_signal_errors() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04",
                "signals": {
                    "notABool": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM inputs",
                        // `expect` compiles fine but evaluates to a number, not a bool, so this
                        // signal errors at evaluation time rather than failing an assertion.
                        "expect": "result"
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let passed = eval(base, EvalOptions::default()).unwrap();
        assert!(!passed, "eval should fail when a signal errors");
    }

    #[test]
    fn channel_with_no_effects_is_an_output_source() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        // No effect sets `channel: tail`, so this channel is a pure output source: its subprocess
        // still runs for the duration of the simulation and its output becomes outputs.
        write_yaml(
            base.join(".rngo/channels/tail.yml"),
            &json!({
                "type": "stream", "command": "printf 'one\\ntwo\\n'"
            }),
        );

        write_yaml(
            base.join(".rngo/signals/tail-outputs.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM outputs WHERE input_id IS NULL AND channel = 'tail'",
                "expect": "result == 2"
            }),
        );

        eval(base, EvalOptions::default()).unwrap();

        let (value, passed) = signal_outcome(base, "tail-outputs");
        assert!(
            passed,
            "expected 2 outputs from the effect-less channel, got {value}"
        );
    }

    #[test]
    fn dry_run_does_not_persist_or_run_channels() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("dry_run_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "echo {{data.id}} >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": { "type": "template", "template": command },
                "type": "exec"
            }),
        );

        let passed = eval(
            base,
            EvalOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(passed, "dry eval of a valid spec should succeed");
        assert!(
            !base.join(".rngo/evals").exists(),
            "dry eval should not create an eval directory"
        );
        assert!(
            !output.exists(),
            "dry eval should not invoke channel side effects"
        );
    }

    #[test]
    fn dry_run_returns_error_when_simulation_cannot_be_built() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        // minimum > maximum is a build-time generator error.
        write_yaml(
            base.join(".rngo/effects/broken.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "generator": {
                    "type": "number",
                    "minimum": 100,
                    "maximum": 18
                }
            }),
        );

        let result = eval(
            base,
            EvalOptions {
                dry_run: true,
                ..Default::default()
            },
        );

        assert!(
            result.is_err(),
            "dry eval should return an error when the simulation can't be built"
        );
        assert!(
            !base.join(".rngo/evals").exists(),
            "a failed dry eval should not create an eval directory either"
        );
    }

    #[test]
    fn dry_run_rejects_invalid_effect_limit() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "limit": 0,
                "generator": { "type": "constant", "value": 1 }
            }),
        );

        let error = eval(
            base,
            EvalOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("effects.ping.limit"),
            "error should name the effect: {error}"
        );
    }

    #[test]
    fn limit_caps_total_effects_produced() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("limit_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-02-01"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, hour)",
                "generator": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "echo {{data.id}} >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": { "type": "template", "template": command },
                "type": "exec"
            }),
        );

        eval(
            base,
            EvalOptions {
                limit: std::num::NonZeroU64::new(3),
                ..Default::default()
            },
        )
        .unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert_eq!(
            content.lines().count(),
            3,
            "limit should cap the eval at exactly 3 effects"
        );
    }

    #[test]
    fn signals_can_read_simulation_timing() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "generator": { "type": "constant", "value": 1 }
            }),
        );

        write_yaml(
            base.join(".rngo/signals/simulation-times.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM metadata WHERE type = 'timing' AND data ->> 'key' IN ('simulation_start', 'simulation_end')",
                "expect": "result == 2"
            }),
        );

        assert!(eval(base, EvalOptions::default()).unwrap());

        let (_, passed) = signal_outcome(base, "simulation-times");
        assert!(
            passed,
            "signals should be able to read the simulation start and end times"
        );
    }

    fn write_timed_spec(base: &Path, end: &str, sent_log: &Path) {
        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/channels")).unwrap();
        fs::create_dir_all(base.join(".rngo/signals")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({ "seed": 1, "start": "now", "end": end }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "channel": "logger",
                "trigger": "hz(1, second)",
                "generator": { "type": "constant", "value": 1 }
            }),
        );

        let command = format!(
            "echo \"{{{{timestamp}}}} $(date +%s)\" >> {}",
            sent_log.to_str().unwrap()
        );
        write_yaml(
            base.join(".rngo/channels/logger.yml"),
            &json!({
                "format": { "type": "template", "template": command },
                "type": "exec"
            }),
        );

        write_yaml(
            base.join(".rngo/signals/held-inputs.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM inputs WHERE timestamp > (SELECT data ->> 'timestamp' FROM metadata WHERE type = 'timing' AND data ->> 'key' = 'simulation_end')"
            }),
        );
    }

    /// Each line is `(input timestamp, wall-clock second it was sent)`.
    fn read_sent_log(path: &Path) -> Vec<(chrono::DateTime<chrono::FixedOffset>, i64)> {
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let (timestamp, sent) = line.split_once(' ').unwrap();
                (
                    chrono::DateTime::parse_from_rfc3339(timestamp).unwrap(),
                    sent.parse().unwrap(),
                )
            })
            .collect()
    }

    fn input_count(base: &Path) -> i64 {
        rusqlite::Connection::open(base.join(".rngo/evals/last/log.sqlite"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM inputs", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn future_inputs_are_sent_once_their_time_arrives() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let sent_log = base.join("sent.txt");
        write_timed_spec(base, "now + seconds(3)", &sent_log);

        eval(
            base,
            EvalOptions {
                realtime: true,
                ..Default::default()
            },
        )
        .unwrap();

        let sent = read_sent_log(&sent_log);
        assert!(!sent.is_empty());
        for (timestamp, sent_at) in &sent {
            assert!(
                timestamp.timestamp() <= *sent_at,
                "input at {timestamp} was sent early, at {sent_at}"
            );
        }
        assert!(sent.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn future_inputs_are_sent_right_away_by_default() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let sent_log = base.join("sent.txt");
        write_timed_spec(base, "now + minutes(10)", &sent_log);

        let started = std::time::Instant::now();
        eval(
            base,
            EvalOptions {
                limit: std::num::NonZeroU64::new(10),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let sent = read_sent_log(&sent_log);
        assert_eq!(sent.len(), 10);
        assert!(
            sent.iter()
                .any(|(timestamp, sent_at)| timestamp.timestamp() > *sent_at),
            "some inputs should have been sent before their time"
        );
    }

    #[test]
    fn stopping_a_waiting_run_still_audits_it() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let sent_log = base.join("sent.txt");
        write_timed_spec(base, "now + minutes(10)", &sent_log);

        let stop = StopHandle::new();
        let stopper = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                stop.stop();
            })
        };

        let started = std::time::Instant::now();
        let succeeded = eval(
            base,
            EvalOptions {
                realtime: true,
                stop,
                ..Default::default()
            },
        )
        .unwrap();
        stopper.join().unwrap();

        assert!(!succeeded, "an interrupted eval reports failure");

        assert!(started.elapsed() < std::time::Duration::from_secs(5));

        let sent = read_sent_log(&sent_log).len() as i64;
        let (held, _) = signal_outcome(base, "held-inputs");
        assert_eq!(held.as_i64().unwrap(), 0, "the waiting input is not logged");
        assert_eq!(
            input_count(base),
            sent,
            "every input logged before the stop was sent"
        );
    }
}
