use rngo_sim::Signal;
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

pub struct SignalCapture {
    handle: JoinHandle<()>,
    tx: Sender<Signal>,
}

impl SignalCapture {
    pub fn start(path: &Path) -> Result<Self, Box<dyn Error>> {
        let (tx, rx) = mpsc::channel::<Signal>();
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let handle = thread::spawn(move || {
            let mut file = file;
            for signal in rx {
                if let Ok(line) = serde_json::to_string(&signal) {
                    let _ = writeln!(file, "{line}");
                }
            }
        });
        Ok(Self { handle, tx })
    }

    pub fn tx(&self) -> Sender<Signal> {
        self.tx.clone()
    }

    pub fn finish(self) -> Result<(), Box<dyn Error>> {
        drop(self.tx);
        self.handle
            .join()
            .map_err(|_| "signal thread panicked".into())
    }
}
