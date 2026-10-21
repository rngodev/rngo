use crate::proxy::channel::ChannelSinkBuilder;
use crate::{ParseError, spec};

pub trait ChannelSinkParser {
    fn key(&self) -> &str;
    fn parse(
        &self,
        channel: &spec::Channel,
    ) -> Result<Box<dyn ChannelSinkBuilder>, Vec<ParseError>>;
}
