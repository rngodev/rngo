mod fs_proxy;
mod simple;
mod sqlite_proxy;

use crate::Signal;
use crate::effect::{Input, SkippedInput};
use std::rc::Rc;

pub use fs_proxy::FsProxyLog;
pub use simple::SimpleEventLog;
pub use sqlite_proxy::SqliteProxyLog;

pub trait Log: std::fmt::Debug {
    fn push(&mut self, event: LogEvent);
    fn reader(&self) -> Rc<dyn LogReader>;
}

pub enum LogEvent {
    Input(Input),
    Skipped(SkippedInput),
    Signal(Signal),
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

impl From<Signal> for LogEvent {
    fn from(s: Signal) -> Self {
        LogEvent::Signal(s)
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
