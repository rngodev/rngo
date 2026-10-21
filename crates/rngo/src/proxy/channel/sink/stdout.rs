use crate::effect::Input;
use crate::parse::{ChannelSinkParser, FormatContext};
use crate::proxy::channel::{ChannelSink, ChannelSinkBuilder};
use crate::proxy::format::Format;
use crate::proxy::output::{OutputSender, SinkOutput};
use crate::{BuildError, ParseError, spec};
use serde_json::Value;
use std::rc::Rc;

#[derive(Debug, Default)]
pub struct Stdout {
    format: Option<Rc<dyn Format>>,
}

impl Stdout {
    pub fn parser() -> StdoutParser {
        StdoutParser {}
    }

    pub fn builder() -> StdoutBuilder {
        StdoutBuilder::default()
    }
}

impl ChannelSink for Stdout {
    fn send(&mut self, input: &Input) -> Result<Vec<SinkOutput>, Box<dyn std::error::Error>> {
        let data = match super::render(self.format.as_deref(), input) {
            Ok(data) => data,
            Err(output) => return Ok(vec![output]),
        };

        let data = match data {
            Value::String(s) => s,
            other => other.to_string(),
        };
        println!("{data}");

        Ok(vec![])
    }
}

#[derive(Default)]
pub struct StdoutBuilder {
    format: Option<Rc<dyn Format>>,
}

impl StdoutBuilder {
    pub fn format(mut self, format: impl Format + 'static) -> Self {
        self.set_format(Rc::new(format));
        self
    }

    pub fn set_format(&mut self, format: Rc<dyn Format>) -> &mut Self {
        self.format = Some(format);
        self
    }
}

impl ChannelSinkBuilder for StdoutBuilder {
    fn build(
        &self,
        _channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn ChannelSink>, Vec<BuildError>> {
        Ok(Box::new(Stdout {
            format: self.format.clone(),
        }))
    }
}

pub struct StdoutParser {}

impl ChannelSinkParser for StdoutParser {
    fn key(&self) -> &str {
        "stdout"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatContext,
    ) -> Result<Box<dyn ChannelSinkBuilder>, Vec<ParseError>> {
        let mut builder = StdoutBuilder::default();

        if let Some(format) = formats.parse(channel)? {
            builder.set_format(format);
        }

        Ok(Box::new(builder))
    }
}
