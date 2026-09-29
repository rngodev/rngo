use crate::parse::ChannelTargetParser;
use crate::proxy::channel::{ChannelTarget, ChannelTargetBuilder};
use crate::{BuildError, Input, Level, Output, ParseError, spec};
use chrono::Utc;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct Exec {
    channel_key: String,
}

impl Exec {
    pub fn parser() -> ExecParser {
        ExecParser {}
    }

    pub fn builder() -> ExecBuilder {
        ExecBuilder
    }
}

impl ChannelTarget for Exec {
    fn send(
        &mut self,
        input: &Input,
        data: Option<String>,
    ) -> Result<Vec<Output>, Box<dyn std::error::Error>> {
        let command = data.ok_or_else(|| {
            format!(
                "channel '{}': exec needs formatted data to run",
                self.channel_key
            )
        })?;

        let output = Command::new("sh")
            .arg("-c")
            .arg(&command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        let timestamp = Utc::now();
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
                    outputs.push(Output {
                        input_id: Some(input.id),
                        channel: self.channel_key.clone(),
                        level,
                        data: line,
                        timestamp,
                        metadata: vec![],
                    });
                }
            }
        }

        if !output.status.success() {
            outputs.push(Output {
                input_id: Some(input.id),
                channel: self.channel_key.clone(),
                level: Level::Error,
                data: format!("command exited with {}", output.status),
                timestamp,
                metadata: vec![],
            });
        }

        Ok(outputs)
    }
}

#[derive(Default)]
pub struct ExecBuilder;

impl ChannelTargetBuilder for ExecBuilder {
    fn build(
        &self,
        channel_key: &str,
        _output_tx: Sender<Output>,
    ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
        Ok(Box::new(Exec {
            channel_key: channel_key.to_string(),
        }))
    }

    fn requires_format(&self) -> bool {
        true
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
                          format instead"
                    .into(),
            }]);
        }

        Ok(Box::new(ExecBuilder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::channel::Channel;
    use crate::proxy::format::TemplateFormat;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::mpsc;

    #[test]
    fn channel_without_a_format_is_an_error() {
        let (tx, _rx) = mpsc::channel();
        let result = Channel::builder("logger".into())
            .target(Exec::builder())
            .output_tx(tx)
            .build();
        assert!(matches!(
            result.unwrap_err().as_slice(),
            [BuildError::Channel { channel, .. }] if channel == "logger"
        ));
    }

    #[test]
    fn runs_the_formatted_data_as_its_command() {
        let (tx, _rx) = mpsc::channel();
        let format = TemplateFormat::new("echo '{{data.s}}'").unwrap();
        let mut channel = Channel::builder("logger".into())
            .format(format)
            .target(Exec::builder())
            .output_tx(tx)
            .build()
            .unwrap();
        let input = Input {
            id: 1,
            effect: "ping".into(),
            offset: 0,
            timestamp: Utc::now().fixed_offset(),
            data: json!({ "s": "<a> & \"b\"" }),
            metadata: vec![],
        };

        let data = channel.format.as_ref().unwrap().format(&input).unwrap();
        let outputs = channel.target.send(&input, Some(data)).unwrap();

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].data, "<a> & \"b\"");
    }
}
