use rngo_sim::{Dialect, Event, spec};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

pub fn run(stdout: bool) -> Result<(), Box<dyn Error>> {
    let spec = load_spec()?;

    let effect_systems: HashMap<String, String> = spec
        .effects
        .iter()
        .filter_map(|(k, v)| v.system.as_ref().map(|s| (k.clone(), s.clone())))
        .collect();

    let system_commands: HashMap<String, String> = spec
        .systems
        .iter()
        .map(|(k, v)| (k.clone(), v.import.command.clone()))
        .collect();

    let run_dir = next_run_dir()?;
    fs::create_dir_all(&run_dir)?;
    fs::write(
        run_dir.join("spec.json"),
        serde_json::to_string_pretty(&spec)?,
    )?;

    let simulation_builder = Dialect::core()
        .parse_simulation(spec)
        .map_err(join_errors)?;

    let simulation = simulation_builder.build().map_err(join_errors)?;

    // Start a subprocess for each system referenced by an effect.
    let mut system_stdinpipes: HashMap<String, ChildStdin> = HashMap::new();
    let mut system_children: HashMap<String, Child> = HashMap::new();

    for system_key in effect_systems.values() {
        if system_stdinpipes.contains_key(system_key) {
            continue;
        }
        let command = system_commands
            .get(system_key)
            .ok_or_else(|| format!("effect references unknown system: {system_key}"))?;
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::piped())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin was piped");
        system_stdinpipes.insert(system_key.clone(), stdin);
        system_children.insert(system_key.clone(), child);
    }

    let mut files: HashMap<String, fs::File> = HashMap::new();

    for event in simulation {
        if stdout {
            println!("{}", serde_json::to_string(&event)?);
        } else {
            match &event {
                Event::Effect { key, .. } => {
                    if let Some(system_key) = effect_systems.get(key) {
                        if let Some(stdin) = system_stdinpipes.get_mut(system_key) {
                            let line = serde_json::to_string(&event)?;
                            writeln!(stdin, "{line}")?;
                        }
                    } else {
                        let line = serde_json::to_string(&event)?;
                        let file = if let Some(f) = files.get_mut(key) {
                            f
                        } else {
                            let path = run_dir.join(format!("{key}.jsonl"));
                            let f = OpenOptions::new().create(true).append(true).open(path)?;
                            files.entry(key.clone()).or_insert(f)
                        };
                        writeln!(file, "{line}")?;
                    }
                }
                Event::Error { message, .. } => {
                    eprintln!("error: {message}");
                }
            }
        }
    }

    // Signal EOF to each system process then wait for it to finish.
    drop(system_stdinpipes);
    for (_, mut child) in system_children {
        child.wait()?;
    }

    Ok(())
}

fn load_spec() -> Result<spec::Simulation, Box<dyn Error>> {
    let spec_path = Path::new(".rngo/spec.yml");

    let mut spec: serde_json::Value = if spec_path.exists() {
        serde_yaml::from_str(&fs::read_to_string(spec_path)?)?
    } else {
        serde_json::json!({ "seed": 1, "effects": {} })
    };

    if !spec["effects"].is_object() {
        spec["effects"] = serde_json::json!({});
    }

    let effects_dir = Path::new(".rngo/effects");
    if effects_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(effects_dir)?
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

    let systems_dir = Path::new(".rngo/systems");
    if systems_dir.is_dir() {
        let mut paths: Vec<_> = fs::read_dir(systems_dir)?
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

    Ok(serde_json::from_value(spec)?)
}

fn next_run_dir() -> Result<PathBuf, Box<dyn Error>> {
    let runs_dir = Path::new(".rngo/runs/local");
    fs::create_dir_all(runs_dir)?;

    let next = fs::read_dir(runs_dir)?
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
