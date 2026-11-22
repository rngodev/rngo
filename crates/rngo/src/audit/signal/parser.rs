use crate::audit::signal::SignalBuilder;
use crate::error::ParseErrors;
use crate::spec;

/// Parses the signals whose `type` is [`SignalParser::key`].
pub trait SignalParser: Send + Sync {
    fn key(&self) -> &str;
    /// Parses `signal` into a signal builder, which the audit builds with the signal's key. Error
    /// paths are relative to the signal, which the dialect prefixes with `signals.<key>`.
    fn parse(&self, signal: &spec::Signal) -> Result<Box<dyn SignalBuilder>, ParseErrors>;
}
