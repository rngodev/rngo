#![warn(unreachable_pub)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_logo_url = "https://raw.githubusercontent.com/rngodev/rngo/main/assets/logo.svg")]
#![doc = include_str!("../README.md")]
mod audit;
pub mod build;
mod cel;
mod effect;
mod error;
mod log;
mod moment;
mod parse;
pub mod parsers;
mod proxy;
mod rng;
pub mod spec;

pub use audit::{Audit, AuditBuilder, AuditReport, signal};
pub use effect::simulation::{Simulation, SimulationBuilder, StandaloneSimulation};
pub use effect::{Input, SkippedInput, Step, generator};
pub use error::{
    BuildError, ChannelError, EffectKey, FormatError, ParseError, ParseErrorKind, ProxyError,
    RunError, RunLogError, SignalError, SimulationKey,
};
pub use log::{Metadata, RunLogReader, RunLogWriter, SimpleEventRunLog, SqliteRunLog};
pub use parse::Dialect;
pub use proxy::output::{Level, Output};
pub use proxy::{Proxy, ProxyBuilder, StopHandle, channel, format};
pub use rng::Rng;
