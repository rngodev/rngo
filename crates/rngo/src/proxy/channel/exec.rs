use crate::effect::Input;
use crate::error::{BuildErrors, ParseErrors};
use crate::proxy::channel::{
    Channel, ChannelBuildContext, ChannelBuilder, ChannelParseContext, ChannelParser, SendContext,
};
use crate::proxy::format::{Format, FormatBuilder};
use crate::proxy::output::ChannelOutput;
use crate::{ChannelError, Level, ParseError, spec};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Exec {
    channel_key: String,
    format: Option<Arc<dyn Format>>,
}

impl Exec {
    pub(crate) fn parser() -> ExecParser {
        ExecParser {}
    }

    pub(crate) fn builder() -> ExecBuilder {
        ExecBuilder::default()
    }
}

impl Channel for Exec {
    fn send(
        &mut self,
        input: &Input,
        context: &SendContext,
    ) -> Result<Vec<ChannelOutput>, ChannelError> {
        let data = match super::render(self.format.as_deref(), input, context) {
            Ok(data) => data,
            Err(output) => return Ok(vec![output]),
        };

        let Value::String(command) = data else {
            return Ok(vec![ChannelOutput::new(
                Level::Error,
                format!("exec needs a string command, got: {data}"),
            )]);
        };

        let output = super::shell(&command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|source| ChannelError::spawn(self.channel_key.clone(), source))?;

        let mut outputs = vec![];

        for (bytes, level) in [
            (&output.stdout, Level::Info),
            (&output.stderr, Level::Error),
        ] {
            for line in BufReader::new(bytes.as_slice())
                .lines()
                .map_while(Result::ok)
            {
                if !line.is_empty() {
                    outputs.push(ChannelOutput::new(level, line));
                }
            }
        }

        if !output.status.success() {
            outputs.push(ChannelOutput::new(
                Level::Error,
                format!("command exited with {}", output.status),
            ));
        }

        Ok(outputs)
    }
}

#[derive(Default)]
pub struct ExecBuilder {
    format: Option<Box<dyn FormatBuilder>>,
}

impl ExecBuilder {
    pub fn format(mut self, format: impl FormatBuilder + 'static) -> Self {
        self.format = Some(Box::new(format));
        self
    }
}

impl ChannelBuilder for ExecBuilder {
    fn build(
        self: Box<Self>,
        context: ChannelBuildContext,
    ) -> Result<Box<dyn Channel>, BuildErrors> {
        let channel_key = context.key();
        Ok(Box::new(Exec {
            channel_key: channel_key.to_string(),
            format: super::build_format(self.format, &context)?,
        }))
    }
}

pub(crate) struct ExecParser {}

impl ChannelParser for ExecParser {
    fn key(&self) -> &str {
        "exec"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        context: &ChannelParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, ParseErrors> {
        if channel.fields.contains_key("command") {
            return Err(ParseError::invalid(
                "exec no longer takes a command; set the channel's format to a template \
                          format, or have the effect produce the command as a string, instead",
            )
            .at(["command"])
            .into());
        }

        let mut builder = ExecBuilder::default();

        if let Some(value) = channel.fields.get("format")
            && let Some(format) = context.parse_format(&["format"], value)?
        {
            builder = builder.format(format);
        }

        Ok(Box::new(builder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::output::OutputSender;
    use jiff::Timestamp;
    use serde_json::json;
    use std::sync::mpsc;

    fn input(data: Value) -> Input {
        Input {
            channel: None,
            id: 1,
            effect: "ping".into(),
            timestamp: Timestamp::now(),
            data,
            metadata: vec![],
        }
    }

    fn exec() -> Box<dyn Channel> {
        let (tx, _rx) = mpsc::channel();
        Box::new(Exec::builder())
            .build(ChannelBuildContext::new(
                "logger",
                OutputSender::new("logger", tx),
            ))
            .unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn runs_string_data_as_its_command() {
        let outputs = exec()
            .send(&input(json!("echo '<a> & \"b\"'")), &SendContext::new())
            .unwrap();
        assert_eq!(
            outputs,
            vec![ChannelOutput::new(Level::Info, "<a> & \"b\"")]
        );
    }

    #[test]
    fn non_string_data_is_an_error_output() {
        let outputs = exec()
            .send(&input(json!({ "a": 1 })), &SendContext::new())
            .unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].level, Level::Error);
    }
}
