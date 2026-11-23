use crate::audit::signal::SignalBuilder;
use crate::error::ParseErrors;
use crate::spec;

/// Parses the signals whose `type` is [`SignalParser::key`].
pub trait SignalParser: Send + Sync {
    fn key(&self) -> &str;
    /// Parses `signal` into a signal builder, which the audit builds with the signal's key. Error
    /// paths are relative to the signal, which the dialect prefixes with `signals.<key>`.
    fn parse(
        &self,
        signal: &spec::Signal,
        context: &SignalParseContext,
    ) -> Result<Box<dyn SignalBuilder>, ParseErrors>;
}

/// What a signal parser sees while parsing a signal.
pub struct SignalParseContext<'a> {
    spec: &'a spec::Spec,
}

impl<'a> SignalParseContext<'a> {
    pub(crate) fn new(spec: &'a spec::Spec) -> Self {
        SignalParseContext { spec }
    }

    /// The whole spec being parsed.
    pub fn spec(&self) -> &'a spec::Spec {
        self.spec
    }
}
