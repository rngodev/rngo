mod simple;
mod sqlite;

use crate::Output;
use crate::effect::{Input, SkippedInput};
use std::rc::Rc;

pub use simple::SimpleEventLog;
pub use sqlite::SqliteLog;

pub trait Log: std::fmt::Debug {
    fn push(&mut self, event: LogEvent);
    fn reader(&self) -> Rc<dyn LogReader>;
}

pub enum LogEvent {
    Input(Input),
    Skipped(SkippedInput),
    Output(Output),
}

impl From<Input> for LogEvent {
    fn from(e: Input) -> Self {
        LogEvent::Input(e)
    }
}

impl From<SkippedInput> for LogEvent {
    fn from(e: SkippedInput) -> Self {
        LogEvent::Skipped(e)
    }
}

impl From<Output> for LogEvent {
    fn from(s: Output) -> Self {
        LogEvent::Output(s)
    }
}

pub trait LogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<Input>>;
    fn index(&self, config: LogIndexConfig) -> Box<dyn LogIndex>;
}

pub trait LogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<Input>>;
}

#[derive(Clone, Debug)]
pub enum LogIndexConfig {
    ByEffect { key: String, last_only: bool },
}
