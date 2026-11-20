use crate::error::ParseErrors;
use crate::proxy::channel::ChannelBuilder;
use crate::proxy::format::FormatParseContext;
use crate::spec;

pub trait ChannelParser {
    fn key(&self) -> &str;

    /// Parses `channel` into a channel builder; `formats` parses its `format` field for channels that
    /// accept one. Error paths are relative to the channel.
    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors>;
}
