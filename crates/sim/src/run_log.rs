mod simple;
mod sqlite;

use crate::Output;
use crate::effect::{Input, SkippedInput};
use crate::schema::Metadata;
use serde_json::Value;
use std::rc::Rc;

pub use simple::SimpleEventRunLog;
pub use sqlite::SqliteRunLog;

/// A run's storage backend. Doesn't expose reads or writes itself - instead it mints
/// independent, cheaply-cloneable handles via [`RunLog::reader`] and [`RunLog::writer`], each
/// sharing the same underlying state. Any number of readers and writers can be minted and held
/// concurrently (e.g. [`crate::System`] writes inputs/outputs while another component writes
/// audit results), the same way multiple effects already hold their own [`RunLogReader`].
pub trait RunLog: std::fmt::Debug {
    fn reader(&self) -> Rc<dyn RunLogReader>;
    fn writer(&self) -> Rc<dyn RunLogWriter>;
}

pub trait RunLogWriter: std::fmt::Debug {
    fn push_input(&self, input: Input);
    fn push_output(&self, output: Output);
    fn push_metadata(&self, metadata: EffectMetadata);
    fn get_signal_value(&self, query: &str) -> Option<Value>;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Last,
    Random,
    Unique,
}

#[derive(Clone, Debug)]
pub struct EffectMetadata {
    input_id: Option<i64>,
    effect: String,
    offset: u64,
    metadata: Vec<Metadata>,
}

/// A skipped occurrence never produces a stored input, so its metadata is always logged with no
/// `input_id` to attach to.
impl From<SkippedInput> for EffectMetadata {
    fn from(skipped: SkippedInput) -> Self {
        EffectMetadata {
            input_id: None,
            effect: skipped.effect,
            offset: skipped.offset,
            metadata: skipped.metadata,
        }
    }
}
