use rngo_sim::{Io, Signal};
use handlebars::Handlebars;
use rngo_sim::spec;
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStderr, ChildStdin, Command, Stdio};

pub struct SystemDispatch {
    effect_systems: HashMap<String, String>,
    stdinpipes: HashMap<String, ChildStdin>,
    stderrpipes: HashMap<String, ChildStderr>,
    children: HashMap<String, Child>,
    hbs: Handlebars<'static>,
}

impl SystemDispatch {
    pub fn new(spec: &spec::Simulation) -> Result<Self, Box<dyn Error>> {
        let effect_systems: HashMap<String, String> = spec
            .effects
            .iter()
            .filter_map(|(k, v)| v.system.as_ref().map(|s| (k.clone(), s.clone())))
            .collect();

        let system_imports: HashMap<String, spec::SystemImport> = spec
            .systems
            .iter()
            .map(|(k, v)| (k.clone(), v.import.clone()))
            .collect();

        let mut stdinpipes = HashMap::new();
        let mut stderrpipes = HashMap::new();
        let mut children = HashMap::new();
        let mut hbs = Handlebars::new();

        for system_key in effect_systems.values() {
            let import = system_imports
                .get(system_key)
                .ok_or_else(|| format!("effect references unknown system: {system_key}"))?;

            match import {
                spec::SystemImport::Stream { command } => {
                    if stdinpipes.contains_key(system_key) {
                        continue;
                    }
                    let mut child = Command::new("sh")
                        .arg("-c")
                        .arg(command)
                        .stdin(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;
                    let stdin = child.stdin.take().expect("stdin was piped");
                    let stderr = child.stderr.take().expect("stderr was piped");
                    stdinpipes.insert(system_key.clone(), stdin);
                    stderrpipes.insert(system_key.clone(), stderr);
                    children.insert(system_key.clone(), child);
                }
                spec::SystemImport::Exec { command } => {
                    hbs.register_template_string(system_key, command)?;
                }
            }
        }

        Ok(Self {
            effect_systems,
            stdinpipes,
            stderrpipes,
            children,
            hbs,
        })
    }

    pub fn send(
        &mut self,
        effect_key: &str,
        value: &serde_json::Value,
        format: Option<&str>,
    ) -> Result<Vec<Signal>, Box<dyn Error>> {
        let system_key = match self.effect_systems.get(effect_key) {
            Some(k) => k.clone(),
            None => return Ok(vec![]),
        };

        if let Some(stdin) = self.stdinpipes.get_mut(&system_key) {
            let data = format
                .map(|f| f.to_string())
                .unwrap_or_else(|| serde_json::to_string(value).unwrap());
            writeln!(stdin, "{data}")?;
            Ok(vec![])
        } else if self.hbs.has_template(&system_key) {
            let command = self.hbs.render(&system_key, value)?;
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()?;
            let effect = Some(effect_key.to_string());
            let mut signals: Vec<Signal> = BufReader::new(output.stdout.as_slice())
                .lines()
                .filter_map(|l| l.ok())
                .filter(|l| !l.is_empty())
                .map(|data| Signal {
                    effect: effect.clone(),
                    system: system_key.clone(),
                    io: Io::Stdout,
                    data,
                })
                .collect();
            let stderr_signals: Vec<Signal> = BufReader::new(output.stderr.as_slice())
                .lines()
                .filter_map(|l| l.ok())
                .filter(|l| !l.is_empty())
                .map(|data| Signal {
                    effect: effect.clone(),
                    system: system_key.clone(),
                    io: Io::Stderr,
                    data,
                })
                .collect();
            signals.extend(stderr_signals);
            Ok(signals)
        } else {
            Ok(vec![])
        }
    }

    pub fn finish(mut self) -> Result<Vec<Signal>, Box<dyn Error>> {
        drop(self.stdinpipes);
        let mut signals = vec![];
        for (system_key, mut child) in self.children {
            child.wait()?;
            if let Some(stderr) = self.stderrpipes.remove(&system_key) {
                let stderr_signals = BufReader::new(stderr)
                    .lines()
                    .filter_map(|l| l.ok())
                    .filter(|l| !l.is_empty())
                    .map(|data| Signal {
                        effect: None,
                        system: system_key.clone(),
                        io: Io::Stderr,
                        data,
                    });
                signals.extend(stderr_signals);
            }
        }
        Ok(signals)
    }
}
