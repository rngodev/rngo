pub(crate) mod sql;

use cel::Program;
pub use sql::SqlSignal;

use rngo_core::BuildError;
use rngo_log::RunLogReader;
use serde::Serialize;

pub trait Signal: std::fmt::Debug {
    fn evaluate(&self, run_log: &dyn RunLogReader) -> SignalOutcome;
}

pub trait SignalBuilder: std::fmt::Debug {
    fn build(self: Box<Self>, key: &str) -> Result<Box<dyn Signal>, Vec<BuildError>>;
}

impl SignalBuilder for Box<dyn Signal> {
    fn build(self: Box<Self>, _key: &str) -> Result<Box<dyn Signal>, Vec<BuildError>> {
        Ok(*self)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
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
pub struct CelExpectation {
    source: String,
    program: Program,
}
