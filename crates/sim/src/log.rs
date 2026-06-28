use crate::Signal;
use crate::effect::EffectEvent;
use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub trait EventLogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<EffectEvent>>;
    fn index(&self, config: EventLogIndexConfig) -> Box<dyn EventLogIndex>;
}

pub trait EventLog: std::fmt::Debug {
    fn push_effect(&mut self, event: &EffectEvent);
    fn push_signal(&mut self, event: &Signal);
    fn push_error(&mut self, event: &str);
    fn reader(&self) -> Rc<dyn EventLogReader>;
}

pub trait EventLogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<EffectEvent>>;
}

#[derive(Clone, Debug)]
pub enum EventLogIndexConfig {
    ByEffect { key: String, last_only: bool },
}

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
    fn push_effect(&mut self, event: &EffectEvent) {
        let file = Self::get_file(&self.directory, &mut self.effect_file, "effects");
        let line = serde_json::to_string(&event).unwrap();
        writeln!(file, "{line}").unwrap();
        self.child.push_effect(event);
    }

    fn push_signal(&mut self, event: &Signal) {
        let file = Self::get_file(&self.directory, &mut self.signal_file, "signal");
        let line = serde_json::to_string(&event).unwrap();
        writeln!(file, "{line}").unwrap();
        self.child.push_signal(event);
    }

    fn push_error(&mut self, event: &str) {
        let file = Self::get_file(&self.directory, &mut self.error_file, "error");
        let line = serde_json::to_string(&event).unwrap();
        writeln!(file, "{line}").unwrap();
        self.child.push_error(event);
    }

    fn reader(&self) -> Rc<dyn EventLogReader> {
        self.child.reader()
    }
}

#[derive(Clone, Debug)]
pub struct SimpleEventLogReader {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
}

impl EventLogReader for SimpleEventLogReader {
    fn last(&self) -> Option<Rc<EffectEvent>> {
        self.effect_events.borrow().last().cloned()
    }

    fn index(&self, config: EventLogIndexConfig) -> Box<dyn EventLogIndex> {
        Box::new(SimpleEventLogIndex {
            effect_events: Rc::clone(&self.effect_events),
            config,
        })
    }
}

#[derive(Default, Debug)]
pub struct SimpleEventLog {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
}

impl EventLog for SimpleEventLog {
    fn push_effect(&mut self, effect_event: &EffectEvent) {
        self.effect_events
            .borrow_mut()
            .push(Rc::new(effect_event.clone()));
    }

    fn push_error(&mut self, _event: &str) {}

    fn push_signal(&mut self, _event: &Signal) {}

    fn reader(&self) -> Rc<dyn EventLogReader> {
        Rc::new(SimpleEventLogReader {
            effect_events: Rc::clone(&self.effect_events),
        })
    }
}

#[derive(Debug)]
pub struct SimpleEventLogIndex {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
    config: EventLogIndexConfig,
}

impl EventLogIndex for SimpleEventLogIndex {
    fn sample(&self) -> Option<Rc<EffectEvent>> {
        let effect_events = self.effect_events.borrow();

        let mut filtered_events = effect_events.iter().filter(|e| match &self.config {
            EventLogIndexConfig::ByEffect {
                key: config_key, ..
            } => &e.key == config_key,
        });

        match &self.config {
            EventLogIndexConfig::ByEffect { last_only, .. } => {
                if *last_only {
                    filtered_events.next_back().cloned()
                } else {
                    let filtered_events = filtered_events.collect::<Vec<_>>();
                    if filtered_events.is_empty() {
                        None
                    } else {
                        let idx = fastrand::usize(..filtered_events.len());
                        filtered_events.get(idx).cloned().cloned()
                    }
                }
            }
        }
    }
}
