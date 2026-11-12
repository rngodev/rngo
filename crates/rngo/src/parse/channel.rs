use crate::parse::FormatParseContext;
use crate::proxy::channel::ChannelBuilder;
use crate::{ParseError, spec};

pub trait ChannelParser {
    fn key(&self) -> &str;

    /// Parses `channel` into a channel builder; `formats` parses its `format` field for channels that
    /// accept one. Error paths are relative to the channel.
    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, Vec<ParseError>>;
}
