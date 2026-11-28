#![warn(unreachable_pub)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_logo_url = "https://raw.githubusercontent.com/rngodev/rngo/main/assets/logo.svg")]
#![doc = include_str!("../README.md")]
mod audit;
pub mod build;
mod cel;
mod dialect;
mod effect;
mod error;
mod log;
mod moment;
pub mod parse;
mod proxy;
mod rng;
pub mod spec;

pub use audit::{Audit, AuditBuilder, AuditReport, signal};
pub use dialect::Dialect;
pub use effect::simulation::{Simulation, SimulationBuilder, StandaloneSimulation};
pub use effect::{Input, SkippedInput, Step, generator};
pub use error::{
    BatchTarget, BuildError, BuildErrors, ChannelError, EffectKey, FormatError, LogError,
    ParseError, ParseErrorKind, ParseErrors, ProxyError, SignalError, SimulationError,
    SimulationKey,
};
pub use log::{LogReader, LogWriter, Metadata, SimpleLog, SqliteLog};
pub use proxy::output::{Level, Output};
pub use proxy::{Proxy, ProxyBuilder, StopHandle, channel, format};
pub use rng::Rng;
