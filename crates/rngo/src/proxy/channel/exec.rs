use crate::effect::Input;
use crate::parse::{ChannelParser, FormatParseContext};
use crate::proxy::channel::{Channel, ChannelBuilder};
use crate::proxy::format::Format;
use crate::proxy::output::{ChannelOutput, OutputSender};
use crate::{BuildError, ChannelError, Level, ParseError, spec};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::rc::Rc;

#[derive(Debug)]
pub struct Exec {
    channel_key: String,
    format: Option<Rc<dyn Format>>,
}

impl Exec {
    pub fn parser() -> ExecParser {
        ExecParser {}
    }

    pub fn builder() -> ExecBuilder {
        ExecBuilder::default()
    }
}

impl Channel for Exec {
    fn send(&mut self, input: &Input) -> Result<Vec<ChannelOutput>, ChannelError> {
        let data = match super::render(self.format.as_deref(), input) {
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
            .map_err(|source| ChannelError::Spawn {
                channel: self.channel_key.clone(),
                source,
            })?;

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
    format: Option<Rc<dyn Format>>,
}

impl ExecBuilder {
    pub fn format(mut self, format: impl Format + 'static) -> Self {
        self.set_format(Rc::new(format));
        self
    }

    pub fn set_format(&mut self, format: Rc<dyn Format>) -> &mut Self {
        self.format = Some(format);
        self
    }
}

impl ChannelBuilder for ExecBuilder {
    fn build(
        &self,
        channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn Channel>, Vec<BuildError>> {
        Ok(Box::new(Exec {
            channel_key: channel_key.to_string(),
            format: self.format.clone(),
        }))
    }
}

pub struct ExecParser {}

impl ChannelParser for ExecParser {
    fn key(&self) -> &str {
        "exec"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
        formats: &FormatParseContext,
    ) -> Result<Box<dyn ChannelBuilder>, Vec<ParseError>> {
        if channel.fields.contains_key("command") {
            return Err(vec![
                ParseError::invalid(
                    "exec no longer takes a command; set the channel's format to a template \
                          format, or have the effect produce the command as a string, instead",
                )
                .at(["command"]),
            ]);
        }

        let mut builder = ExecBuilder::default();

        if let Some(value) = channel.fields.get("format")
            && let Some(format) = formats.parse(&["format"], value)?
        {
            builder.set_format(format);
        }

        Ok(Box::new(builder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;

    fn input(data: Value) -> Input {
        Input {
            id: 1,
            effect: "ping".into(),
            timestamp: chrono::Utc::now().fixed_offset(),
            data,
            metadata: vec![],
        }
    }

    fn exec() -> Box<dyn Channel> {
        let (tx, _rx) = mpsc::channel();
        Exec::builder()
            .build("logger", OutputSender::new("logger", tx))
            .unwrap()
    }

    #[test]
    fn runs_string_data_as_its_command() {
        let outputs = exec().send(&input(json!("echo '<a> & \"b\"'"))).unwrap();
        assert_eq!(
            outputs,
            vec![ChannelOutput::new(Level::Info, "<a> & \"b\"")]
        );
    }

    #[test]
    fn non_string_data_is_an_error_output() {
        let outputs = exec().send(&input(json!({ "a": 1 }))).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].level, Level::Error);
    }
}
