pub(crate) mod sql;

use crate::error::BuildErrors;
use cel::Program;
pub(crate) use sql::SqlSignal;

mod parser;
pub use parser::{SignalParseContext, SignalParser};

use crate::LogReader;
use serde::Serialize;

/// A check run once against a finished run's log.
pub trait Signal: std::fmt::Debug + Send {
    fn evaluate(&self, context: &SignalContext) -> SignalOutcome;
}

/// What a signal sees when it is evaluated.
pub struct SignalContext<'a> {
    log: &'a dyn LogReader,
}

impl<'a> SignalContext<'a> {
    pub(crate) fn new(log: &'a dyn LogReader) -> Self {
        SignalContext { log }
    }

    /// The finished run's log.
    pub fn log(&self) -> &'a dyn LogReader {
        self.log
    }
}

/// Builds a [`Signal`] when its audit is built. Every `Signal` is its own builder.
pub trait SignalBuilder: std::fmt::Debug + Send {
    fn build(self: Box<Self>, context: SignalBuildContext) -> Result<Box<dyn Signal>, BuildErrors>;
}

/// What a signal builder gets when its audit is built.
pub struct SignalBuildContext<'a> {
    key: &'a str,
}

impl<'a> SignalBuildContext<'a> {
    pub(crate) fn new(key: &'a str) -> Self {
        SignalBuildContext { key }
    }

    /// The signal's key, to name it in build errors.
    pub fn key(&self) -> &'a str {
        self.key
    }
}

impl SignalBuilder for Box<dyn SignalBuilder> {
    fn build(self: Box<Self>, context: SignalBuildContext) -> Result<Box<dyn Signal>, BuildErrors> {
        (*self).build(context)
    }
}

impl<S: Signal + 'static> SignalBuilder for S {
    fn build(
        self: Box<Self>,
        _context: SignalBuildContext,
    ) -> Result<Box<dyn Signal>, BuildErrors> {
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
