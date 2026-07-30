use chrono::Utc;
use rngo_sim::{Level, Signal, spec};
use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

/// How long to give a signal source's subprocess to exit on its own before it's forcibly killed.
/// A subprocess that finishes right as the simulation does (e.g. a quick one-shot command) can
/// otherwise be killed before the OS has even scheduled it to run, losing its entire output.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Runs each non-interactive signal source's subprocess for the lifetime of the simulation,
/// turning its stdout/stderr lines into [`Signal`]s associated with a system but no effect.
pub struct SignalDispatch {
    children: Vec<Child>,
    reader_threads: Vec<thread::JoinHandle<()>>,
}

impl SignalDispatch {
    pub fn new(spec: &spec::Simulation, signal_tx: Sender<Signal>) -> Result<Self, Box<dyn Error>> {
        let mut children = vec![];
        let mut reader_threads = vec![];

        for (key, signal) in &spec.signals {
            let spec::SignalExport::Stream { command } = &signal.export;

            let mut child = Command::new("sh")
                .arg("-c")
                .arg(command)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;

            if let Some(stdout) = child.stdout.take() {
                let tx = signal_tx.clone();
                let system = signal.system.clone();
                let key = key.clone();
                reader_threads.push(thread::spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        if let Ok(data) = line
                            && !data.is_empty()
                        {
                            let _ = tx.send(Signal {
                                effect_id: None,
                                key: Some(key.clone()),
                                system: system.clone(),
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
                let system = signal.system.clone();
                let key = key.clone();
                reader_threads.push(thread::spawn(move || {
                    for line in BufReader::new(stderr).lines() {
                        if let Ok(data) = line
                            && !data.is_empty()
                        {
                            let _ = tx.send(Signal {
                                effect_id: None,
                                key: Some(key.clone()),
                                system: system.clone(),
                                level: Level::Error,
                                data,
                                timestamp: Utc::now(),
                            });
                        }
                    }
                }));
            }

            children.push(child);
        }

        Ok(Self {
            children,
            reader_threads,
        })
    }

    /// Kills each subprocess, since non-interactive signal sources (e.g. `tail -f`) have no
    /// natural end of their own, then joins the reader threads so any output already written
    /// before the kill has been turned into a `Signal` before the caller does a final drain.
    pub fn finish(mut self) -> Result<(), Box<dyn Error>> {
        for child in &mut self.children {
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
        for mut child in self.children {
            let _ = child.wait();
        }
        for handle in self.reader_threads {
            let _ = handle.join();
        }
        Ok(())
    }
}
