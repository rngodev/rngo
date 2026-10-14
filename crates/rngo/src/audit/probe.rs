pub(crate) mod sql;

use cel::Program;
pub use sql::SqlProbe;

use crate::RunLogReader;
use crate::build::BuildError;
use serde::Serialize;

pub trait Probe: std::fmt::Debug {
    fn evaluate(&self, run_log: &dyn RunLogReader) -> ProbeOutcome;
}

pub trait ProbeBuilder: std::fmt::Debug {
    fn build(self: Box<Self>, key: &str) -> Result<Box<dyn Probe>, Vec<BuildError>>;
}

impl ProbeBuilder for Box<dyn Probe> {
    fn build(self: Box<Self>, _key: &str) -> Result<Box<dyn Probe>, Vec<BuildError>> {
        Ok(*self)
    }
}

/// What a probe produced: its signals (possibly none), or the error that kept it from running.
#[derive(Clone, Debug)]
pub enum ProbeOutcome {
    Signals(Vec<Signal>),
    Error { error: String },
}

/// One result of a probe: the value it observed and, when the probe has an expectation, whether it held.
#[derive(Clone, Debug, Serialize)]
pub struct Signal {
    pub value: serde_json::Value,
    pub eval: Option<SignalEval>,
}

impl Signal {
    /// True unless the signal has an expectation that failed.
    pub fn passed(&self) -> bool {
        self.eval.as_ref().is_none_or(|eval| eval.passed)
    }
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
