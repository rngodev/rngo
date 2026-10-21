use crate::parse::ChannelSinkParser;
use crate::proxy::channel::{ChannelSink, ChannelSinkBuilder};
use crate::proxy::output::{OutputSender, SinkOutput};
use crate::{BuildError, Level, ParseError, spec};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::Stdio;

#[derive(Debug)]
pub struct Exec;

impl Exec {
    pub fn parser() -> ExecParser {
        ExecParser {}
    }

    pub fn builder() -> ExecBuilder {
        ExecBuilder
    }
}

impl ChannelSink for Exec {
    fn send(&mut self, data: Value) -> Result<Vec<SinkOutput>, Box<dyn std::error::Error>> {
        let Value::String(command) = data else {
            return Ok(vec![SinkOutput::new(
                Level::Error,
                format!("exec needs a string command, got: {data}"),
            )]);
        };

        let output = super::shell(&command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

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
                    outputs.push(SinkOutput::new(level, line));
                }
            }
        }

        if !output.status.success() {
            outputs.push(SinkOutput::new(
                Level::Error,
                format!("command exited with {}", output.status),
            ));
        }

        Ok(outputs)
    }
}

#[derive(Default)]
pub struct ExecBuilder;

impl ChannelSinkBuilder for ExecBuilder {
    fn build(
        &self,
        _channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn ChannelSink>, Vec<BuildError>> {
        Ok(Box::new(Exec))
    }
}

pub struct ExecParser {}

impl ChannelSinkParser for ExecParser {
    fn key(&self) -> &str {
        "exec"
    }

    fn parse(
        &self,
        channel: &spec::Channel,
    ) -> Result<Box<dyn ChannelSinkBuilder>, Vec<ParseError>> {
        if channel.fields.contains_key("command") {
            return Err(vec![ParseError::Invalid {
                path: Some(vec!["command".into()]),
                message: "exec no longer takes a command; set the channel's format to a template \
                          format, or have the effect produce the command as a string, instead"
                    .into(),
            }]);
        }

        Ok(Box::new(ExecBuilder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;

    fn exec() -> Box<dyn ChannelSink> {
        let (tx, _rx) = mpsc::channel();
        Exec::builder()
            .build("logger", OutputSender::new("logger", tx))
            .unwrap()
    }

    #[test]
    fn runs_string_data_as_its_command() {
        let outputs = exec().send(json!("echo '<a> & \"b\"'")).unwrap();
        assert_eq!(outputs, vec![SinkOutput::new(Level::Info, "<a> & \"b\"")]);
    }

    #[test]
    fn non_string_data_is_an_error_output() {
        let outputs = exec().send(json!({ "a": 1 })).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].level, Level::Error);
    }
}
