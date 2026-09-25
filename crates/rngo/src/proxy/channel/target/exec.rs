use crate::parse::ChannelTargetParser;
use crate::proxy::channel::{ChannelTarget, ChannelTargetBuilder};
use crate::{BuildError, Input, Level, Output, ParseError, spec};
use chrono::Utc;
use handlebars::Handlebars;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct Exec {
    channel_key: String,
    hbs: Handlebars<'static>,
}

impl Exec {
    pub fn parser() -> ExecParser {
        ExecParser {}
    }

    pub fn builder() -> ExecBuilder {
        ExecBuilder::default()
    }
}

impl ChannelTarget for Exec {
    fn send(
        &mut self,
        input: &Input,
        _data: Option<String>,
    ) -> Result<Vec<Output>, Box<dyn std::error::Error>> {
        let command = self.hbs.render("command", &input.data)?;

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
pub struct ExecBuilder {
    command: Option<String>,
}

impl ExecBuilder {
    pub fn command(mut self, value: impl Into<String>) -> Self {
        self.set_command(value);
        self
    }

    pub fn set_command(&mut self, value: impl Into<String>) -> &mut Self {
        self.command = Some(value.into());
        self
    }
}

impl ChannelTargetBuilder for ExecBuilder {
    fn build(
        &self,
        channel_key: &str,
        _output_tx: Sender<Output>,
    ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
        let Some(command) = self.command.clone() else {
            return Err(vec![BuildError::ChannelTarget {
                channel: channel_key.to_string(),
                message: "command not specified".into(),
            }]);
        };

        let mut hbs = Handlebars::new();
        hbs.register_template_string("command", &command)
            .map_err(|_e| {
                vec![BuildError::ChannelTarget {
                    channel: channel_key.to_string(),
                    message: "FIX ME must be a string".into(),
                }]
            })?;

        Ok(Box::new(Exec {
            channel_key: channel_key.to_string(),
            hbs,
        }))
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
        let command = match channel_target.fields.get("command") {
            Some(value) => match value.as_str() {
                Some(command) => command,
                None => {
                    return Err(vec![ParseError::SchemaError {
                        path: None,
                        message: "FIX ME must be a string".into(),
                    }]);
                }
            },
            None => {
                return Err(vec![ParseError::SchemaError {
                    path: None,
                    message: "FIX ME must be a string".into(),
                }]);
            }
        };

        Ok(Box::new(ExecBuilder::default().command(command)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn builder_without_a_command_is_an_error() {
        let (tx, _rx) = mpsc::channel();
        let result = ExecBuilder::default().build("logger", tx);
        assert!(result.is_err());
    }

    #[test]
    fn builder_with_no_channel_key_at_construction_still_builds() {
        let (tx, _rx) = mpsc::channel();
        let result = Exec::builder().command("echo hi").build("logger", tx);
        assert!(result.is_ok());
    }
}
