use crate::parse::ChannelTargetParser;
use crate::proxy::channel::{ChannelTarget, ChannelTargetBuilder};
use crate::{BuildError, Level, OutputSender, ParseError, TargetOutput, spec};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

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

impl ChannelTarget for Exec {
    fn send(&mut self, data: Value) -> Result<Vec<TargetOutput>, Box<dyn std::error::Error>> {
        let Value::String(command) = data else {
            return Ok(vec![TargetOutput::new(
                Level::Error,
                format!("exec needs a string command, got: {data}"),
            )]);
        };

        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
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
                    outputs.push(TargetOutput::new(level, line));
                }
            }
        }

        if !output.status.success() {
            outputs.push(TargetOutput::new(
                Level::Error,
                format!("command exited with {}", output.status),
            ));
        }

        Ok(outputs)
    }
}

#[derive(Default)]
pub struct ExecBuilder;

impl ChannelTargetBuilder for ExecBuilder {
    fn build(
        &self,
        _channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
        Ok(Box::new(Exec))
    }
}

pub struct ExecParser {}

impl ChannelTargetParser for ExecParser {
    fn key(&self) -> &str {
        "exec"
    }

    fn parse(
        &self,
        channel_target: &spec::ChannelTarget,
    ) -> Result<Box<dyn ChannelTargetBuilder>, Vec<ParseError>> {
        if channel_target.fields.contains_key("command") {
            return Err(vec![ParseError::SchemaError {
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

    fn exec() -> Box<dyn ChannelTarget> {
        let (tx, _rx) = mpsc::channel();
        Exec::builder()
            .build("logger", OutputSender::new("logger", tx))
            .unwrap()
    }

    #[test]
    fn runs_string_data_as_its_command() {
        let outputs = exec().send(json!("echo '<a> & \"b\"'")).unwrap();
        assert_eq!(outputs, vec![TargetOutput::new(Level::Info, "<a> & \"b\"")]);
    }

    #[test]
    fn non_string_data_is_an_error_output() {
        let outputs = exec().send(json!({ "a": 1 })).unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].level, Level::Error);
    }
}
