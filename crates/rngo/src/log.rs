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

#[derive(Clone, Debug)]
pub struct Metadata {
    pub mtype: String,
    pub input_id: Option<i64>,
    pub output_id: Option<i64>,
    pub data: Option<Value>,
    pub segment: Option<String>,
    /// When the metadata's subject occurred in simulation time, e.g. a skipped attempt.
    pub timestamp: Option<Timestamp>,
}
