mod simple;
mod sqlite;

use crate::Output;
use crate::effect::Input;
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
    fn push_metadata(&self, metadata: Metadata);
}

pub trait RunLogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<Input>>;

    /// Runs a backend-specific query string against the log, returning the single scalar column
    /// of its first row - or `None` if this backend can't answer it (e.g. [`SimpleEventRunLog`],
    /// which has no query engine behind it) or the query itself produced no result.
    fn query(&self, query: &str) -> Option<Value>;

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

/// One row of the standalone `metadata` table (see `run_log/sqlite.rs`) - for metadata with no
/// input/output row of its own to be embedded in directly (e.g. a skipped occurrence, or an
/// audit signal's result). Named `Metadata` rather than `EffectMetadata` since it's no longer
/// effect-specific; distinct from [`crate::schema::Metadata`], the per-entry description that
/// this type's `data` (for a skipped occurrence) or `inputs`/`outputs`' own `metadata` column
/// embed. Mirrors the table's columns (minus `type`, renamed `mtype` to dodge the keyword) - only
/// `type` itself is `NOT NULL`. Has no `effect` field: a row with an `input_id`/`output_id` can
/// already recover it via a join, and a row with no id at all (e.g. a skipped occurrence, see
/// `effect.rs`) folds it into `data` instead.
#[derive(Clone, Debug)]
pub struct Metadata {
    pub mtype: String,
    pub input_id: Option<i64>,
    pub output_id: Option<i64>,
    pub offset: Option<u64>,
    pub data: Option<Value>,
    pub segment: Option<String>,
}
