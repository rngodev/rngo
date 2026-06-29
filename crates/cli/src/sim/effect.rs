use chrono::Utc;
use handlebars::Handlebars;
use rngo_sim::{EffectEvent, Io, Signal, spec};
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

pub struct EffectDispatch {
    effect_systems: HashMap<String, String>,
    stdinpipes: HashMap<String, ChildStdin>,
    children: HashMap<String, Child>,
    hbs: Handlebars<'static>,
    signal_tx: Sender<Signal>,
}

impl EffectDispatch {
    pub fn new(spec: &spec::Simulation, signal_tx: Sender<Signal>) -> Result<Self, Box<dyn Error>> {
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
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;

                    let stdin = child.stdin.take().expect("stdin was piped");
                    stdinpipes.insert(system_key.clone(), stdin);

                    if let Some(stdout) = child.stdout.take() {
                        let tx = signal_tx.clone();
                        let system_key = system_key.clone();
                        thread::spawn(move || {
                            for line in BufReader::new(stdout).lines() {
                                if let Ok(data) = line
                                    && !data.is_empty()
                                {
                                    let _ = tx.send(Signal {
                                        effect_id: None,
                                        system: system_key.clone(),
                                        io: Io::Stdout,
                                        data,
                                        timestamp: Utc::now(),
                                    });
                                }
                            }
                        });
                    }

                    if let Some(stderr) = child.stderr.take() {
                        let tx = signal_tx.clone();
                        let system_key = system_key.clone();
                        thread::spawn(move || {
                            for line in BufReader::new(stderr).lines() {
                                if let Ok(data) = line
                                    && !data.is_empty()
                                {
                                    eprintln!("[{system_key}] {data}");
                                    let _ = tx.send(Signal {
                                        effect_id: None,
                                        system: system_key.clone(),
                                        io: Io::Stderr,
                                        data,
                                        timestamp: Utc::now(),
                                    });
                                }
                            }
                        });
                    }

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
            children,
            hbs,
            signal_tx,
        })
    }

    pub fn send(&mut self, effect_event: &EffectEvent) -> Result<(), Box<dyn Error>> {
        let system_key = match self.effect_systems.get(&effect_event.key) {
            Some(k) => k.clone(),
            None => return Ok(()),
        };

        if let Some(stdin) = self.stdinpipes.get_mut(&system_key) {
            let data = effect_event
                .format
                .as_ref()
                .map(|f| f.to_string())
                .unwrap_or_else(|| serde_json::to_string(&effect_event.value).unwrap());
            writeln!(stdin, "{data}")
                .map_err(|e| format!("system '{system_key}': {e}"))?;
        } else if self.hbs.has_template(&system_key) {
            let command = self.hbs.render(&system_key, &effect_event.value)?;
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()?;

            let timestamp = Utc::now();
            for (bytes, io) in [(&output.stdout, Io::Stdout), (&output.stderr, Io::Stderr)] {
                for line in BufReader::new(bytes.as_slice())
                    .lines()
                    .map_while(Result::ok)
                {
                    if !line.is_empty() {
                        let _ = self.signal_tx.send(Signal {
                            effect_id: Some(effect_event.id),
                            system: system_key.clone(),
                            io,
                            data: line,
                            timestamp,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    pub fn finish(self) -> Result<(), Box<dyn Error>> {
        drop(self.stdinpipes);
        for (_, mut child) in self.children {
            child.wait()?;
        }
        Ok(())
    }
}
