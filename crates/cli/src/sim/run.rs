use crate::sim::status::StatusLog;
use crate::sim::system::SystemDispatch;
use console::style;
use rngo_sim::{Dialect, SimpleEventLog, SqliteProxyLog, spec};
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::{fmt, fs};
use uuid::Uuid;

pub fn run(base: &Path, stdout: bool, spec_path: Option<&Path>) -> Result<bool, Box<dyn Error>> {
    let _ = dotenvy::from_path(base.join(".env"));

    let spec = match spec_path {
        Some(path) => load_spec_file(path)?,
        None => load_spec(base)?,
    };

    let run_dir = new_run_dir(base)?;
    fs::create_dir_all(&run_dir)?;
    fs::write(
        run_dir.join("spec.json"),
        serde_json::to_string_pretty(&spec)?,
    )?;
    update_last_symlink(base, &run_dir)?;

    let effect_systems: HashMap<String, String> = spec
        .effects
        .iter()
        .filter_map(|(k, v)| v.system.as_ref().map(|s| (k.clone(), s.clone())))
        .collect();
    let log = StatusLog::new(
        Box::new(SqliteProxyLog::new(
            Box::new(SimpleEventLog::default()),
            run_dir.clone(),
        )),
        effect_systems,
    );

    let simulation_builder = Dialect::primitive()
        .parse_simulation(spec.clone())
        .map_err(join_errors)?;

    let mut simulation = simulation_builder.log(log).build().map_err(join_errors)?;
    let systems = simulation.take_systems();
    let mut system_dispatch = SystemDispatch::new(&spec, systems, simulation.signal_tx())?;

    for effect_event in &mut simulation {
        if stdout {
            println!("{}", serde_json::to_string(&effect_event)?);
        } else {
            system_dispatch.send(&effect_event)?;
        }
    }

    // Closes stdin on every stream system (triggering exit for those that react to EOF) and
    // kills any stragglers - including signal-source systems with no natural end - after a
    // grace period; `simulation.finish()` drains the trailing signals this produces and drops
    // the simulation, committing the event log before invariants are evaluated below.
    system_dispatch.finish()?;
    simulation.finish();

    let mut all_passed = true;

    if !spec.invariants.is_empty() {
        let outcomes =
            rngo_sim::invariant::evaluate_from_log(&run_dir.join("log.sqlite"), &spec.invariants)
                .map_err(join_errors)?;
        fs::write(
            run_dir.join("invariants.json"),
            serde_json::to_string_pretty(&outcomes)?,
        )?;

        println!();
        println!("{}", style("Audit").bold());
        let passed = outcomes.values().filter(|o| o.passed).count();
        println!("{passed}/{} invariants passed", outcomes.len());

        for (key, outcome) in &outcomes {
            if outcome.passed {
                continue;
            }
            all_passed = false;
            let spec::Invariant::Sql { expect, .. } = &spec.invariants[key];
            println!("{key} failed - got {}, expected '{expect}'", outcome.value);
        }
    }

    Ok(all_passed)
}

fn load_spec_file(path: &Path) -> Result<spec::Simulation, Box<dyn Error>> {
    let value: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    Ok(spec::from_value(value).map_err(join_errors)?)
}

fn load_spec(base: &Path) -> Result<spec::Simulation, Box<dyn Error>> {
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

    if !spec["systems"].is_object() {
        spec["systems"] = serde_json::json!({});
    }

    let systems_dir = base.join(".rngo/systems");
    if systems_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&systems_dir)?
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
            let system: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["systems"][key] = system;
        }
    }

    if !spec["schemas"].is_object() {
        spec["schemas"] = serde_json::json!({});
    }

    let schemas_dir = base.join(".rngo/schemas");
    if schemas_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&schemas_dir)?
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
            let schema: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["schemas"][key] = schema;
        }
    }

    if !spec["invariants"].is_object() {
        spec["invariants"] = serde_json::json!({});
    }

    let invariants_dir = base.join(".rngo/invariants");
    if invariants_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(&invariants_dir)?
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
            let invariant: serde_json::Value = serde_yaml::from_str(&fs::read_to_string(&path)?)?;
            spec["invariants"][key] = invariant;
        }
    }

    Ok(spec::from_value(spec).map_err(join_errors)?)
}

fn new_run_dir(base: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let runs_dir = base.join(".rngo/runs");
    fs::create_dir_all(&runs_dir)?;
    let id = Uuid::now_v7();
    Ok(runs_dir.join(id.to_string()))
}

fn update_last_symlink(base: &Path, run_dir: &Path) -> Result<(), Box<dyn Error>> {
    let link = base.join(".rngo/runs/last");
    if link.exists() || link.is_symlink() {
        fs::remove_file(&link)?;
    }
    let target = run_dir.strip_prefix(base.join(".rngo/runs"))?;
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

    #[test]
    fn exec_import_runs_command_per_event() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("exec_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/systems")).unwrap();

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
                "system": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "echo {{id}} >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/systems/logger.yml"),
            &json!({
                "format": {},
                "import": { "type": "exec", "command": command }
            }),
        );

        run(base, false, None).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "exec command should have run once per event"
        );
    }

    #[test]
    fn exec_import_records_signal_when_command_fails() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/systems")).unwrap();
        fs::create_dir_all(base.join(".rngo/invariants")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/invariants/has-failure-signal.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM signals WHERE data LIKE 'command exited with%'",
                "expect": "result >= 1"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "system": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/systems/logger.yml"),
            &json!({
                "format": {},
                "import": { "type": "exec", "command": "exit 1" }
            }),
        );

        run(base, false, None).unwrap();

        let invariants_path = base.join(".rngo/runs/last/invariants.json");
        let content = fs::read_to_string(&invariants_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(value["has-failure-signal"]["passed"], true);
    }

    #[test]
    fn stream_import_does_not_drop_trailing_signal() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/systems")).unwrap();
        fs::create_dir_all(base.join(".rngo/invariants")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        // Every event fed to `cat` is echoed straight back out over stdout, so the number of
        // signals recorded should match the number of effects exactly - including the one
        // produced by the very last event, which arrives only after the subprocess is closed.
        write_yaml(
            base.join(".rngo/invariants/matches-effect-count.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT (SELECT COUNT(*) FROM effects) - (SELECT COUNT(*) FROM signals)",
                "expect": "result == 0"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "system": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/systems/logger.yml"),
            &json!({
                "format": {},
                "import": { "type": "stream", "command": "cat" }
            }),
        );

        run(base, false, None).unwrap();

        let invariants_path = base.join(".rngo/runs/last/invariants.json");
        let content = fs::read_to_string(&invariants_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            value["matches-effect-count"]["passed"], true,
            "expected signal count to match effect count, got {value}"
        );
    }

    #[test]
    fn stream_import_pipes_events_to_subprocess() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("stream_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/systems")).unwrap();

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
                "system": "logger",
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/systems/logger.yml"),
            &json!({
                "format": {},
                "import": { "type": "stream", "command": command }
            }),
        );

        run(base, false, None).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "stream subprocess should have received events"
        );
    }

    #[test]
    fn sql_format_uses_table_from_effect_metadata() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("sql_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/systems")).unwrap();

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
                "system": "db",
                "metadata": { "table": "accounts" },
                "trigger": "hz(1, hour)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/systems/db.yml"),
            &json!({
                "format": { "type": "sql" },
                "import": { "type": "stream", "command": command }
            }),
        );

        run(base, false, None).unwrap();

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
                "systems": {
                    "logger": {
                        "format": {},
                        "import": { "type": "stream", "command": command }
                    }
                },
                "effects": {
                    "ping": {
                        "system": "logger",
                        "trigger": "hz(1, day)",
                        "schema": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                            }
                        }
                    }
                }
            }),
        );

        run(base, false, Some(&spec_path)).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "run should use the spec file passed via --spec"
        );
    }

    #[test]
    fn schemas_dir_yml_files_are_referenceable_by_type_name() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();
        let output = base.join("schemas_output.txt");

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/systems")).unwrap();
        fs::create_dir_all(base.join(".rngo/schemas")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-02"
            }),
        );

        write_yaml(
            base.join(".rngo/schemas/title.yml"),
            &json!({
                "schema": {
                    "type": "select",
                    "options": [
                        { "schema": { "type": "constant", "value": "Mr." } },
                        { "schema": { "type": "constant", "value": "Mrs." } },
                        { "schema": { "type": "constant", "value": "Dr." } }
                    ]
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "system": "logger",
                "trigger": "hz(1, hour)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "title" }
                    }
                }
            }),
        );

        let command = "cat >> ".to_string() + output.to_str().unwrap();
        write_yaml(
            base.join(".rngo/systems/logger.yml"),
            &json!({
                "format": {},
                "import": { "type": "stream", "command": command }
            }),
        );

        run(base, false, None).unwrap();

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
    fn invariants_are_evaluated_and_written_to_run_dir() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04",
                "invariants": {
                    "hasEvents": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM effects",
                        "expect": "result >= 1"
                    },
                    "tooMany": {
                        "type": "sql",
                        "query": "SELECT COUNT(*) FROM effects",
                        "expect": "result > 1000"
                    }
                }
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        run(base, false, None).unwrap();

        let invariants_path = base.join(".rngo/runs/last/invariants.json");
        let content = fs::read_to_string(&invariants_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(value["hasEvents"]["passed"], true);
        assert!(value["hasEvents"]["value"].as_i64().unwrap() >= 1);
        assert_eq!(value["tooMany"]["passed"], false);
    }

    #[test]
    fn invariants_dir_yml_files_are_merged_into_spec() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/invariants")).unwrap();

        write_yaml(
            base.join(".rngo/spec.yml"),
            &json!({
                "seed": 1,
                "start": "2024-01-01",
                "end": "2024-01-04"
            }),
        );

        write_yaml(
            base.join(".rngo/invariants/has-events.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM effects",
                "expect": "result >= 1"
            }),
        );

        write_yaml(
            base.join(".rngo/effects/ping.yml"),
            &json!({
                "trigger": "hz(1, day)",
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        run(base, false, None).unwrap();

        let invariants_path = base.join(".rngo/runs/last/invariants.json");
        let content = fs::read_to_string(&invariants_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(value["has-events"]["passed"], true);
    }

    #[test]
    fn system_with_no_effects_is_a_signal_source() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        fs::create_dir_all(base.join(".rngo/effects")).unwrap();
        fs::create_dir_all(base.join(".rngo/systems")).unwrap();
        fs::create_dir_all(base.join(".rngo/invariants")).unwrap();

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
                "schema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "number", "minimum": 1, "scale": 0, "step": 1 }
                    }
                }
            }),
        );

        // No effect sets `system: tail`, so this system is a pure signal source: its subprocess
        // still runs for the duration of the simulation and its output becomes signals.
        write_yaml(
            base.join(".rngo/systems/tail.yml"),
            &json!({
                "import": { "type": "stream", "command": "printf 'one\\ntwo\\n'" }
            }),
        );

        write_yaml(
            base.join(".rngo/invariants/tail-signals.yml"),
            &json!({
                "type": "sql",
                "query": "SELECT COUNT(*) FROM signals WHERE effect_id IS NULL AND system = 'tail'",
                "expect": "result == 2"
            }),
        );

        run(base, false, None).unwrap();

        let invariants_path = base.join(".rngo/runs/last/invariants.json");
        let content = fs::read_to_string(&invariants_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(
            value["tail-signals"]["passed"], true,
            "expected 2 signals from the effect-less system, got {value}"
        );
    }
}
