mod pool;
mod simple;
mod sqlite;

use crate::effect::Input;
use crate::{Output, Rng, RunLogError};
use jiff::Timestamp;
use serde_json::Value;
use std::sync::Arc;

pub use simple::SimpleEventRunLog;
pub use sqlite::SqliteRunLog;

pub trait RunLogWriter: std::fmt::Debug + Send + Sync {
    fn push_input(&self, input: Input) -> Result<(), RunLogError>;
    fn push_output(&self, output: Output) -> Result<(), RunLogError>;
    fn push_metadata(&self, metadata: Metadata) -> Result<(), RunLogError>;
    /// Persists any buffered writes; a no-op for writers that don't buffer.
    fn flush(&self) -> Result<(), RunLogError> {
        Ok(())
    }
}

/// Lookups return `None` when nothing matches. A log that fails to read also returns `None`, and
/// reports the failure from its next write or flush instead.
pub trait RunLogReader: std::fmt::Debug + Send + Sync {
    fn last(&self) -> Option<Arc<Input>>;
    fn last_for_effect(&self, key: &str) -> Option<Arc<Input>>;
    fn random_for_effect(&self, key: &str, rng: &mut Rng) -> Option<Arc<Input>>;
    fn unique_for_effect(&self, key: &str, cursor: &str, rng: &mut Rng) -> Option<Arc<Input>>;
    /// Runs `query` against the log, returning its scalar result.
    fn query(&self, query: &str) -> Result<Value, RunLogError>;
}

/// A run log row about the run itself or one of its inputs or outputs, such as a skipped attempt
/// or a signal outcome.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Metadata {
    pub kind: String,
    pub input_id: Option<u64>,
    pub output_id: Option<u64>,
    pub data: Option<Value>,
    pub segment: Option<String>,
    /// When the metadata's subject occurred in simulation time, e.g. a skipped attempt.
    pub timestamp: Option<Timestamp>,
}

impl Metadata {
    /// Metadata of type `kind`, with every other field unset.
    pub fn new(kind: impl Into<String>) -> Self {
        Metadata {
            kind: kind.into(),
            input_id: None,
            output_id: None,
            data: None,
            segment: None,
            timestamp: None,
        }
    }

    /// Ties it to an input.
    pub fn input_id(mut self, input_id: u64) -> Self {
        self.input_id = Some(input_id);
        self
    }

    /// Ties it to an output.
    pub fn output_id(mut self, output_id: u64) -> Self {
        self.output_id = Some(output_id);
        self
    }

    pub fn data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    pub fn segment(mut self, segment: impl Into<String>) -> Self {
        self.segment = Some(segment.into());
        self
    }

    /// When its subject occurred in simulation time.
    pub fn timestamp(mut self, timestamp: Timestamp) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
}
