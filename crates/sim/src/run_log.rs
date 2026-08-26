mod simple;
mod sqlite;

use crate::Output;
use crate::effect::{Input, SkippedInput};
use std::rc::Rc;

pub use simple::SimpleEventRunLog;
pub use sqlite::SqliteRunLog;

pub trait RunLog: std::fmt::Debug {
    fn push(&mut self, event: RunLogEvent);
    fn reader(&self) -> Rc<dyn RunLogReader>;
}

pub enum RunLogEvent {
    Input(Input),
    Skipped(SkippedInput),
    Output(Output),
}

impl From<Input> for RunLogEvent {
    fn from(e: Input) -> Self {
        RunLogEvent::Input(e)
    }
}

impl From<SkippedInput> for RunLogEvent {
    fn from(e: SkippedInput) -> Self {
        RunLogEvent::Skipped(e)
    }
}

impl From<Output> for RunLogEvent {
    fn from(s: Output) -> Self {
        RunLogEvent::Output(s)
    }
}

pub trait RunLogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<Input>>;
    fn index(&self, config: RunLogIndexConfig) -> Box<dyn RunLogIndex>;
}

pub trait RunLogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<Input>>;
}

#[derive(Clone, Debug)]
pub enum RunLogIndexConfig {
    ByEffect { key: String, last_only: bool },
}
