use crate::audit::signal::Signal;
use crate::error::ParseErrors;
use crate::spec;

/// Builds the signals whose `type` is [`SignalParser::key`].
pub trait SignalParser: Send + Sync {
    fn key(&self) -> &str;
    /// Parses signal `key`; error paths are relative to the signal, which the dialect prefixes with `signals.<key>`.
    fn parse(&self, key: &str, signal: &spec::Signal) -> Result<Box<dyn Signal>, ParseErrors>;
}
