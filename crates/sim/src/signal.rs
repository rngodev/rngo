pub(crate) mod sql;

pub use sql::SqlSignal;

use crate::RunLog;
use serde::Serialize;

/// A built, runtime-evaluable signal - the result of a [`crate::parse::SignalParser`] parsing a
/// [`crate::spec::Signal`]. Mirrors [`crate::format::Format`]: parsing is a single step directly
/// to this runtime trait, since (unlike schemas) evaluating a signal needs no persistent
/// build-time resource beyond the [`RunLog`] it's handed on each call.
pub trait Signal: std::fmt::Debug {
    fn evaluate(&self, run_log: &dyn RunLog) -> SignalOutcome;
}

/// A signal with no `expect` still has a `value`, just no `passed` verdict.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SignalOutcome {
    Success {
        value: serde_json::Value,
        passed: Option<bool>,
    },
    Error {
        error: String,
    },
}
