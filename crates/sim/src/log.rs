mod fs_proxy;
mod simple;
mod sqlite_proxy;

use crate::Signal;
use crate::effect::{EffectEvent, SkippedEffectEvent};
use std::rc::Rc;

pub use fs_proxy::FsProxyLog;
pub use simple::SimpleEventLog;
pub use sqlite_proxy::SqliteProxyLog;

pub trait Log: std::fmt::Debug {
    fn push(&mut self, event: LogEvent);
    fn reader(&self) -> Rc<dyn LogReader>;
}

pub enum LogEvent {
    Effect(EffectEvent),
    Skipped(SkippedEffectEvent),
    Signal(Signal),
}

impl From<EffectEvent> for LogEvent {
    fn from(e: EffectEvent) -> Self {
        LogEvent::Effect(e)
    }
}

impl From<SkippedEffectEvent> for LogEvent {
    fn from(e: SkippedEffectEvent) -> Self {
        LogEvent::Skipped(e)
    }
}

impl From<Signal> for LogEvent {
    fn from(s: Signal) -> Self {
        LogEvent::Signal(s)
    }
}

pub trait LogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<EffectEvent>>;
    fn index(&self, config: LogIndexConfig) -> Box<dyn LogIndex>;
}

pub trait LogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<EffectEvent>>;
}

#[derive(Clone, Debug)]
pub enum LogIndexConfig {
    ByEffect { key: String, last_only: bool },
}
