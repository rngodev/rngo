mod simple;
mod sqlite;

use crate::Output;
use crate::effect::Input;
use rand_pcg::Pcg32;
use serde_json::Value;
use std::rc::Rc;

pub use simple::SimpleEventRunLog;
pub use sqlite::SqliteRunLog;

pub trait RunLogWriter: std::fmt::Debug {
    fn push_input(&self, input: Input);
    fn push_output(&self, output: Output);
    fn push_metadata(&self, metadata: Metadata);
}

pub trait RunLogReader: std::fmt::Debug {
    fn last(&self) -> Option<Rc<Input>>;

    /// The most recent input for a single effect - the read-only lookup `Trigger::Effect` polls
    /// to fire a trigger-by-effect.
    fn last_for_effect(&self, key: &str) -> Option<Rc<Input>>;

    /// Runs a backend-specific query string against the log, returning the single scalar column
    /// of its first row - or `None` if this backend can't answer it (e.g. [`SimpleEventRunLog`],
    /// which has no query engine behind it) or the query itself produced no result.
    fn query(&self, query: &str) -> Option<Value>;

    /// A uniformly-random input for a single effect, backing `Reference`'s `cursor: random`.
    fn random_for_effect(&self, key: &str, rng: &mut Pcg32) -> Option<Rc<Input>>;

    /// A uniformly-random input for a single effect, excluding any previously returned under
    /// `cursor` - or `None` once every matching input has been returned. `cursor` scopes this
    /// "already returned" bookkeeping, so two independent `unique` references over the same
    /// effect don't share it; the caller picks one that's stable across its own repeated calls
    /// but distinct from any other caller's (e.g. `SchemaBuildVisitor::path_id`).
    fn unique_for_effect(&self, key: &str, cursor: &str, rng: &mut Pcg32) -> Option<Rc<Input>>;
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
