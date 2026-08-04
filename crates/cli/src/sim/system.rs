use chrono::Utc;
use handlebars::Handlebars;
use rngo_sim::{EffectEvent, Format, Level, Signal, System, spec};
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

/// How long to give a system's subprocess to exit on its own (e.g. after its stdin is closed)
/// before it's forcibly killed. Some systems (e.g. a `tail -F` used as a signal source) never
/// exit on their own, so this bounds shutdown; a subprocess that finishes right as the
/// simulation does can otherwise be killed before the OS has even scheduled it to run, losing
/// its entire output.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Spawns each `stream` system's subprocess for the lifetime of the simulation and each `exec`
/// system's Handlebars command template, then dispatches effect events to the system an effect
/// writes to. A system with no effects writing to it (no `system: <key>` reference) is still
/// spawned if it's a `stream` system, turning its stdout/stderr into signals with no associated
/// effect - a non-interactive signal source.
pub struct SystemDispatch {
    effect_systems: HashMap<String, String>,
    formats: HashMap<String, Box<dyn Format>>,
    stdinpipes: HashMap<String, ChildStdin>,
    children: HashMap<String, Child>,
    reader_threads: Vec<thread::JoinHandle<()>>,
    hbs: Handlebars<'static>,
    signal_tx: Sender<Signal>,
}

impl SystemDispatch {
    pub fn new(
        spec: &spec::Simulation,
        systems: Vec<System>,
        signal_tx: Sender<Signal>,
    ) -> Result<Self, Box<dyn Error>> {
        let effect_systems: HashMap<String, String> = spec
            .effects
            .iter()
            .filter_map(|(k, v)| v.system.as_ref().map(|s| (k.clone(), s.clone())))
            .collect();

        for system_key in effect_systems.values() {
            if !systems.iter().any(|s| &s.key == system_key) {
                return Err(format!("effect references unknown system: {system_key}").into());
            }
        }

        let mut formats: HashMap<String, Box<dyn Format>> = HashMap::new();
        let mut stdinpipes = HashMap::new();
        let mut children = HashMap::new();
        let mut reader_threads = vec![];
        let mut hbs = Handlebars::new();

        for system in systems {
            let System {
                key: system_key,
                format,
                import,
            } = system;

            if let Some(format) = format {
                formats.insert(system_key.clone(), format);
            }

            match import {
                spec::SystemImport::Stream { command } => {
                    let mut child = Command::new("sh")
                        .arg("-c")
                        .arg(&command)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()?;

                    let stdin = child.stdin.take().expect("stdin was piped");
                    stdinpipes.insert(system_key.clone(), stdin);

                    if let Some(stdout) = child.stdout.take() {
                        let tx = signal_tx.clone();
                        let system_key = system_key.clone();
                        reader_threads.push(thread::spawn(move || {
                            for line in BufReader::new(stdout).lines() {
                                if let Ok(data) = line
                                    && !data.is_empty()
                                {
                                    let _ = tx.send(Signal {
                                        effect_id: None,
                                        system: system_key.clone(),
                                        level: Level::Info,
                                        data,
                                        timestamp: Utc::now(),
                                    });
                                }
                            }
                        }));
                    }

                    if let Some(stderr) = child.stderr.take() {
                        let tx = signal_tx.clone();
                        let system_key = system_key.clone();
                        reader_threads.push(thread::spawn(move || {
                            for line in BufReader::new(stderr).lines() {
                                if let Ok(data) = line
                                    && !data.is_empty()
                                {
                                    let _ = tx.send(Signal {
                                        effect_id: None,
                                        system: system_key.clone(),
                                        level: Level::Error,
                                        data,
                                        timestamp: Utc::now(),
                                    });
                                }
                            }
                        }));
                    }

                    children.insert(system_key.clone(), child);
                }
                spec::SystemImport::Exec { command } => {
                    hbs.register_template_string(&system_key, &command)?;
                }
            }
        }

        Ok(Self {
            effect_systems,
            formats,
            stdinpipes,
            children,
            reader_threads,
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
            let data = match self.formats.get(&system_key) {
                Some(format) => format
                    .format(effect_event)
                    .map_err(|e| format!("system '{system_key}': {e}"))?,
                None => serde_json::to_string(&effect_event.value).unwrap(),
            };
            writeln!(stdin, "{data}").map_err(|e| format!("system '{system_key}': {e}"))?;
        } else if self.hbs.has_template(&system_key) {
            let command = self.hbs.render(&system_key, &effect_event.value)?;
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()?;

            let timestamp = Utc::now();
            for (bytes, level) in [
                (&output.stdout, Level::Info),
                (&output.stderr, Level::Error),
            ] {
                for line in BufReader::new(bytes.as_slice())
                    .lines()
                    .map_while(Result::ok)
                {
                    if !line.is_empty() {
                        let _ = self.signal_tx.send(Signal {
                            effect_id: Some(effect_event.id),
                            system: system_key.clone(),
                            level,
                            data: line,
                            timestamp,
                        });
                    }
                }
            }

            if !output.status.success() {
                let _ = self.signal_tx.send(Signal {
                    effect_id: Some(effect_event.id),
                    system: system_key.clone(),
                    level: Level::Error,
                    data: format!("command exited with {}", output.status),
                    timestamp,
                });
            }
        }

        Ok(())
    }

    /// Closes every system's stdin, which triggers exit for subprocesses that react to EOF (e.g.
    /// `cat`), then gives each remaining child a grace period before killing it - covering
    /// signal-source subprocesses (e.g. `tail -F`) that never exit on their own. Reader threads
    /// are joined last so trailing output has already become a `Signal` before the caller does a
    /// final drain of the signal channel.
    pub fn finish(mut self) -> Result<(), Box<dyn Error>> {
        drop(self.stdinpipes);

        for child in self.children.values_mut() {
            let deadline = Instant::now() + SHUTDOWN_GRACE;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) if Instant::now() >= deadline => break,
                    Ok(None) => thread::sleep(SHUTDOWN_POLL_INTERVAL),
                }
            }
            let _ = child.kill();
        }
        for (_, mut child) in self.children {
            let _ = child.wait();
        }
        for handle in self.reader_threads {
            let _ = handle.join();
        }
        Ok(())
    }
}
