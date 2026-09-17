pub(crate) mod sql;

use cel::Program;
pub use sql::SqlSignal;

use crate::RunLogReader;
use crate::build::BuildError;
use serde::Serialize;

/// A built, runtime-evaluable signal - the result of a `SignalParser` parsing a
/// [`crate::spec::Signal`]. Mirrors [`crate::format::Format`]: parsing is a single step directly
/// to this runtime trait, since (unlike schemas) evaluating a signal needs no persistent
/// build-time resource beyond the [`RunLogReader`] it's handed on each call.
pub trait Signal: std::fmt::Debug {
    fn evaluate(&self, run_log: &dyn RunLogReader) -> SignalOutcome;
}

/// The Rust-DSL counterpart to `SignalParser`: builds a [`Signal`] directly from values set
/// through a fluent builder (e.g. [`sql`](crate::build::sql)) instead of from a parsed
/// [`crate::spec::Signal`]. Takes the signal's key at build time, mirroring
/// [`crate::audit::AuditBuilder::with_signal`], which is where that key comes from. Consumes
/// itself, like every other builder in the crate besides [`crate::schema::SchemaBuilder`] (which
/// is rebuilt once per effect that references it) - a signal builder is only ever built once.
pub trait SignalBuilder: std::fmt::Debug {
    fn build(self: Box<Self>, key: &str) -> Result<Box<dyn Signal>, Vec<BuildError>>;
}

/// An already-built signal is trivially its own builder - lets [`crate::parse::Dialect::parse_audit`]
/// feed spec-parsed signals (which `SignalParser` resolves directly, with no builder step of
/// their own) into [`crate::audit::AuditBuilder::set_signal`] alongside Rust-DSL signal builders.
impl SignalBuilder for Box<dyn Signal> {
    fn build(self: Box<Self>, _key: &str) -> Result<Box<dyn Signal>, Vec<BuildError>> {
        Ok(*self)
    }
}

/// A signal with no `expect` still has a `value`, just no `passed` verdict.
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
