use crate::log::LogReader;
use crate::schema::Metadata;
use crate::{Log, LogEvent};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug)]
pub struct FsProxyLog {
    child: Box<dyn Log>,
    directory: PathBuf,
    effect_file: Option<std::fs::File>,
    metadata_file: Option<std::fs::File>,
    signal_file: Option<std::fs::File>,
}

impl FsProxyLog {
    pub fn new(child: Box<dyn Log>, directory: PathBuf) -> Self {
        FsProxyLog {
            child,
            directory,
            effect_file: None,
            metadata_file: None,
            signal_file: None,
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

    fn write_metadata(&mut self, effect_id: Option<u64>, metadata: &[Metadata]) {
        if metadata.is_empty() {
            return;
        }

        let file = Self::get_file(&self.directory, &mut self.metadata_file, "metadata");
        for metadata in metadata {
            let line = serde_json::to_string(&MetadataLine {
                effect_id,
                metadata,
            })
            .unwrap();
            writeln!(file, "{line}").unwrap();
        }
    }
}

impl Log for FsProxyLog {
    fn push(&mut self, event: LogEvent) {
        match &event {
            LogEvent::Effect(e) => {
                let file = Self::get_file(&self.directory, &mut self.effect_file, "effects");
                let line = serde_json::to_string(e).unwrap();
                writeln!(file, "{line}").unwrap();

                self.write_metadata(Some(e.id), &e.metadata);
            }
            LogEvent::Skipped(e) => {
                self.write_metadata(None, &e.metadata);
            }
            LogEvent::Signal(s) => {
                let file = Self::get_file(&self.directory, &mut self.signal_file, "signal");
                let line = serde_json::to_string(s).unwrap();
                writeln!(file, "{line}").unwrap();
            }
        }
        self.child.push(event);
    }

    fn reader(&self) -> Rc<dyn LogReader> {
        self.child.reader()
    }
}

#[derive(Serialize)]
struct MetadataLine<'a> {
    effect_id: Option<u64>,
    #[serde(flatten)]
    metadata: &'a Metadata,
}
