use crate::channel::ChannelTargetBuilder;
use crate::format::Format;
use rngo_core::{ParseError, spec};

pub trait ChannelTargetParser {
    fn key(&self) -> &str;
    fn parse(
        &self,
        channel_target: &spec::ChannelTarget,
    ) -> Result<Box<dyn ChannelTargetBuilder>, Vec<ParseError>>;
}

pub trait FormatParser {
    fn key(&self) -> &str;
    fn parse(
        &self,
        format: &spec::Format,
        spec: &spec::Spec,
    ) -> Result<Box<dyn Format>, Vec<ParseError>>;
}
