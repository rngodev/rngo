use crate::log::EventLogReader;
use crate::{EventLog, LogEvent};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug)]
pub struct FsProxyLog {
    child: Box<dyn EventLog>,
    directory: PathBuf,
    effect_file: Option<std::fs::File>,
    signal_file: Option<std::fs::File>,
    error_file: Option<std::fs::File>,
}

impl FsProxyLog {
    pub fn new(child: Box<dyn EventLog>, directory: PathBuf) -> Self {
        FsProxyLog {
            child,
            directory,
            effect_file: None,
            signal_file: None,
            error_file: None,
        }
    }

    fn get_file<'a>(
        directory: &'a Path,
        file: &'a mut Option<std::fs::File>,
        name: &'a str,
    ) -> &'a mut std::fs::File {
        file.get_or_insert_with(|| {
            let path = directory.join(format!("{name}.jsonl"));
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap()
        })
    }
}

impl EventLog for FsProxyLog {
    fn push(&mut self, event: LogEvent) {
        match &event {
            LogEvent::Effect(e) => {
                let file = Self::get_file(&self.directory, &mut self.effect_file, "effects");
                let line = serde_json::to_string(e).unwrap();
                writeln!(file, "{line}").unwrap();
            }
            LogEvent::Signal(s) => {
                let file = Self::get_file(&self.directory, &mut self.signal_file, "signal");
                let line = serde_json::to_string(s).unwrap();
                writeln!(file, "{line}").unwrap();
            }
            LogEvent::Error(e) => {
                let file = Self::get_file(&self.directory, &mut self.error_file, "error");
                let line = serde_json::to_string(e).unwrap();
                writeln!(file, "{line}").unwrap();
            }
        }
        self.child.push(event);
    }

    fn reader(&self) -> Rc<dyn EventLogReader> {
        self.child.reader()
    }
}
