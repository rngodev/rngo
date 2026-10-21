use crate::parse::FormatContext;
use crate::proxy::channel::ChannelSinkBuilder;
use crate::{ParseError, spec};

pub trait ChannelSinkParser {
    fn key(&self) -> &str;

    /// Parses `channel` into a sink builder; `formats` parses its `format` field for sinks that
    /// accept one. Error paths are relative to the channel.
    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatContext,
    ) -> Result<Box<dyn ChannelSinkBuilder>, Vec<ParseError>>;
}
