use crate::sim::effect::EffectDispatch;
use rngo_sim::{Dialect, FsProxyLog, SimpleEventLog, spec};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::{fmt, fs};

pub fn run(base: &Path, stdout: bool) -> Result<(), Box<dyn Error>> {
    let _ = dotenvy::from_path(base.join(".env"));

    let spec = load_spec(base)?;

    let run_dir = next_run_dir(base)?;
    fs::create_dir_all(&run_dir)?;
    fs::write(
        run_dir.join("spec.json"),
        serde_json::to_string_pretty(&spec)?,
    )?;

    let log = FsProxyLog::new(Box::new(SimpleEventLog::default()), run_dir.clone());

    let simulation_builder = Dialect::core()
        .parse_simulation(spec.clone())
        .map_err(join_errors)?;

    let simulation = simulation_builder.log(log).build().map_err(join_errors)?;
    let mut effect_dispatch = EffectDispatch::new(&spec, simulation.signal_tx())?;

    for effect_event in simulation {
        if stdout {
            println!("{}", serde_json::to_string(&effect_event)?);
        } else {
            effect_dispatch.send(&effect_event)?;
        }
    }

    effect_dispatch.finish()?;

    Ok(())
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

    Ok(spec::from_value(spec).map_err(join_errors)?)
}

fn next_run_dir(base: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let runs_dir = base.join(".rngo/runs/local");
    fs::create_dir_all(&runs_dir)?;

    let next = fs::read_dir(&runs_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<u64>().ok()))
        .max()
        .map(|n| n + 1)
        .unwrap_or(1);

    Ok(runs_dir.join(next.to_string()))
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
                        "id": { "type": "number", "min": 1, "scale": 0, "step": 1 }
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

        run(base, false).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "exec command should have run once per event"
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
                        "id": { "type": "number", "min": 1, "scale": 0, "step": 1 }
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

        run(base, false).unwrap();

        let content = fs::read_to_string(&output).unwrap();
        assert!(
            content.lines().count() > 0,
            "stream subprocess should have received events"
        );
    }
}
