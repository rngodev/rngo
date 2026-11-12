//! Core simulation library for rngo.
//!
//! The supported public API is the re-exports below plus the `build`, `generator`, `spec` and `error` modules;
//! see the README for the list. Everything else is private.
#![warn(unreachable_pub)]
mod audit;
pub mod build;
mod cel;
mod effect;
pub mod error;
mod log;
mod moment;
mod parse;
mod proxy;
pub mod spec;

pub use audit::signal::SignalOutcome;
pub use audit::{Audit, AuditBuilder, AuditReport};
pub use effect::simulation::{Simulation, SimulationBuilder, StandaloneSimulation};
pub use effect::{Input, SkippedInput, Step, generator};
pub use error::{
    BuildError, ChannelError, EffectKey, FormatError, ParseError, ParseErrorKind, ProxyError,
    RunError, RunLogError, SignalError, SimulationKey,
};
pub use log::{Metadata, RunLogReader, RunLogWriter, SimpleEventRunLog, SqliteRunLog};
pub use parse::Dialect;
pub use proxy::output::{Level, Output};
pub use proxy::{Proxy, ProxyBuilder, StopHandle};
