mod simple;
mod sqlite;

use crate::effect::{Input, SkippedInput};
use crate::signal::SignalOutcome;
use crate::{Output, spec};
use indexmap::IndexMap;
use std::rc::Rc;

pub use simple::SimpleEventRunLog;
pub use sqlite::SqliteRunLog;

pub trait RunLog: std::fmt::Debug {
    fn push(&mut self, event: RunLogEvent);
    fn reader(&self) -> Rc<dyn RunLogReader>;

    /// Evaluates `signals` against this run log's recorded events, returning one outcome per
    /// signal keyed the same way. A backend with no query engine over its events (e.g.
    /// [`SimpleEventRunLog`]) reports every signal as unsupported rather than failing outright.
    fn evaluate_signals(
        &self,
        signals: &IndexMap<String, spec::Signal>,
    ) -> IndexMap<String, SignalOutcome>;
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
    ByEffect { key: String, cursor: Cursor },
}

/// How a [`RunLogIndex`] picks which of an effect's prior inputs to return from
/// [`RunLogIndex::sample`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    /// Always the most recently emitted matching input (used by trigger-by-effect).
    Last,
    /// Any matching input, possibly repeating ones already returned (the `reference` schema's
    /// default).
    Random,
    /// Any matching input not yet returned by this index; `None` once exhausted.
    Unique,
}
