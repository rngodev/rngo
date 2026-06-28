mod fs_proxy;
mod simple;

use crate::Signal;
use crate::effect::EffectEvent;
use std::rc::Rc;

pub use fs_proxy::FsProxyLog;
pub use simple::SimpleEventLog;

pub enum LogEvent {
    Effect(EffectEvent),
    Signal(Signal),
    Error(String),
}

impl From<EffectEvent> for LogEvent {
    fn from(e: EffectEvent) -> Self {
        LogEvent::Effect(e)
    }
}

impl From<Signal> for LogEvent {
    fn from(s: Signal) -> Self {
        LogEvent::Signal(s)
    }
}

impl From<String> for LogEvent {
    fn from(s: String) -> Self {
        LogEvent::Error(s)
    }
}

impl From<&str> for LogEvent {
    fn from(s: &str) -> Self {
        LogEvent::Error(s.to_string())
    }
}

pub trait EventLogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<EffectEvent>>;
    fn index(&self, config: EventLogIndexConfig) -> Box<dyn EventLogIndex>;
}

pub trait EventLog: std::fmt::Debug {
    fn push(&mut self, event: LogEvent);
    fn reader(&self) -> Rc<dyn EventLogReader>;
}

pub trait EventLogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<EffectEvent>>;
}

#[derive(Clone, Debug)]
pub enum EventLogIndexConfig {
    ByEffect { key: String, last_only: bool },
}
