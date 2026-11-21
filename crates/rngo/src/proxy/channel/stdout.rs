use crate::effect::Input;
use crate::error::{BuildErrors, ParseErrors};
use crate::proxy::channel::{Channel, ChannelBuilder, ChannelParser};
use crate::proxy::format::{Format, FormatBuilder, FormatParseContext};
use crate::proxy::output::{ChannelOutput, OutputSender};
use crate::{ChannelError, spec};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Default)]
pub(crate) struct Stdout {
    format: Option<Arc<dyn Format>>,
}

impl Stdout {
    pub(crate) fn parser() -> StdoutParser {
        StdoutParser {}
    }

    pub(crate) fn builder() -> StdoutBuilder {
        StdoutBuilder::default()
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
pub struct StdoutBuilder {
    format: Option<Box<dyn FormatBuilder>>,
}

impl StdoutBuilder {
    pub fn format(mut self, format: impl FormatBuilder + 'static) -> Self {
        self.format = Some(Box::new(format));
        self
    }
}

impl ChannelBuilder for StdoutBuilder {
    fn build(
        self: Box<Self>,
        channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn Channel>, BuildErrors> {
        Ok(Box::new(Stdout {
            format: super::build_format(self.format, channel_key)?,
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
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors> {
        let mut builder = StdoutBuilder::default();

        if let Some(value) = channel.fields.get("format")
            && let Some(format) = formats.parse(&["format"], value)?
        {
            builder = builder.format(format);
        }

        Ok(Box::new(builder))
    }
}
