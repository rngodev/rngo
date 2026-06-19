use rngo_sim::{Dialect, Event};
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn run(stdout: bool) -> Result<(), Box<dyn Error>> {
    let spec = load_spec()?;

    let run_dir = next_run_dir()?;
    fs::create_dir_all(&run_dir)?;
    fs::write(run_dir.join("spec.json"), serde_json::to_string_pretty(&spec)?)?;

    let simulation_builder = Dialect::core()
        .parse_simulation_json(spec)
        .map_err(join_errors)?;

    let simulation = simulation_builder.build().map_err(join_errors)?;

    let mut files: HashMap<String, fs::File> = HashMap::new();

    for event in simulation {
        let line = serde_json::to_string(&event)?;

        if stdout {
            println!("{line}");
        } else {
            match &event {
                Event::Effect { key, .. } => {
                    let file = if let Some(f) = files.get_mut(key) {
                        f
                    } else {
                        let path = run_dir.join(format!("{key}.jsonl"));
                        let f = OpenOptions::new().create(true).append(true).open(path)?;
                        files.entry(key.clone()).or_insert(f)
                    };
                    writeln!(file, "{line}")?;
                }
                Event::Error { message, .. } => {
                    eprintln!("error: {message}");
                }
            }
        }
    }

    Ok(())
}

fn load_spec() -> Result<serde_json::Value, Box<dyn Error>> {
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

    Ok(spec)
}

fn next_run_dir() -> Result<PathBuf, Box<dyn Error>> {
    let runs_dir = Path::new(".rngo/runs/local");
    fs::create_dir_all(runs_dir)?;

    let next = fs::read_dir(runs_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|s| s.parse::<u64>().ok())
        })
        .max()
        .map(|n| n + 1)
        .unwrap_or(1);

    Ok(runs_dir.join(next.to_string()))
}

fn join_errors<E: fmt::Display>(errors: Vec<E>) -> String {
    errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n")
}
