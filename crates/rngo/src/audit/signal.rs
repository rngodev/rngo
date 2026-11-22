pub(crate) mod sql;

use crate::error::BuildErrors;
use cel::Program;
pub(crate) use sql::SqlSignal;

mod parser;
pub use parser::SignalParser;

use crate::RunLogReader;
use serde::Serialize;

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

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SignalOutcome {
    Success {
        value: serde_json::Value,
        eval: Option<SignalEval>,
    },
    Error {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalEval {
    pub expectation: String,
    pub passed: bool,
}

#[derive(Debug)]
pub(crate) struct CelExpectation {
    source: String,
    program: Program,
}
