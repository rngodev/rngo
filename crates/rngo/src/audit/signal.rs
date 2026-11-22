pub(crate) mod sql;

use crate::error::BuildErrors;
use cel::Program;
pub(crate) use sql::SqlSignal;

mod parser;
pub use parser::SignalParser;

use crate::RunLogReader;
use serde::Serialize;

/// A check run once against a finished run's log.
pub trait Signal: std::fmt::Debug + Send {
    fn evaluate(&self, run_log: &dyn RunLogReader) -> SignalOutcome;
}

/// Builds a [`Signal`] when its audit is built; `key` names the signal in build errors. Every
/// `Signal` is its own builder.
pub trait SignalBuilder: std::fmt::Debug + Send {
    fn build(self: Box<Self>, key: &str) -> Result<Box<dyn Signal>, BuildErrors>;
}

impl SignalBuilder for Box<dyn SignalBuilder> {
    fn build(self: Box<Self>, key: &str) -> Result<Box<dyn Signal>, BuildErrors> {
        (*self).build(key)
    }
}

impl<S: Signal + 'static> SignalBuilder for S {
    fn build(self: Box<Self>, _key: &str) -> Result<Box<dyn Signal>, BuildErrors> {
        Ok(self)
    }
}

/// The result of evaluating a signal. Build one with [`SignalOutcome::success`] or
/// [`SignalOutcome::error`].
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SignalOutcome {
    /// The signal produced `value`, checked against its expectation in `eval` if it has one.
    #[non_exhaustive]
    Success {
        value: serde_json::Value,
        eval: Option<SignalEval>,
    },
    /// The signal couldn't be evaluated.
    #[non_exhaustive]
    Error { error: String },
}

impl SignalOutcome {
    /// A signal that produced `value`; `eval` is its expectation check, `None` if it has none.
    pub fn success(value: serde_json::Value, eval: Option<SignalEval>) -> Self {
        SignalOutcome::Success { value, eval }
    }

    /// A signal that couldn't be evaluated, and why.
    pub fn error(message: impl Into<String>) -> Self {
        SignalOutcome::Error {
            error: message.into(),
        }
    }
}

/// How a signal's result fared against its expectation.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct SignalEval {
    pub expectation: String,
    pub passed: bool,
}

impl SignalEval {
    /// `expectation` describes what was checked, e.g. the `expect` expression.
    pub fn new(expectation: impl Into<String>, passed: bool) -> Self {
        SignalEval {
            expectation: expectation.into(),
            passed,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CelExpectation {
    source: String,
    program: Program,
}
