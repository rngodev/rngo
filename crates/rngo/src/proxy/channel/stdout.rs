use crate::effect::Input;
use crate::parse::{ChannelParser, FormatParseContext};
use crate::proxy::channel::{Channel, ChannelBuilder};
use crate::proxy::format::Format;
use crate::proxy::output::{ChannelOutput, OutputSender};
use crate::{BuildError, ChannelError, ParseError, spec};
use serde_json::Value;
use std::rc::Rc;

#[derive(Debug, Default)]
pub(crate) struct Stdout {
    format: Option<Rc<dyn Format>>,
}

impl Stdout {
    pub(crate) fn parser() -> StdoutParser {
        StdoutParser {}
    }
}

impl Channel for Stdout {
    fn send(&mut self, input: &Input) -> Result<Vec<ChannelOutput>, ChannelError> {
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
pub(crate) struct StdoutBuilder {
    format: Option<Rc<dyn Format>>,
}

impl StdoutBuilder {
    pub(crate) fn set_format(&mut self, format: Rc<dyn Format>) -> &mut Self {
        self.format = Some(format);
        self
    }
}

impl ChannelBuilder for StdoutBuilder {
    fn build(
        &self,
        _channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn Channel>, Vec<BuildError>> {
        Ok(Box::new(Stdout {
            format: self.format.clone(),
        }))
    }
}

pub(crate) struct StdoutParser {}

impl ChannelParser for StdoutParser {
    fn key(&self) -> &str {
        "stdout"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, Vec<ParseError>> {
        let mut builder = StdoutBuilder::default();

        if let Some(value) = channel.fields.get("format")
            && let Some(format) = formats.parse(&["format"], value)?
        {
            builder.set_format(format);
        }

        Ok(Box::new(builder))
    }
}
