use crate::BuildError;
use crate::proxy::channel::{ChannelSink, ChannelSinkBuilder};
use crate::proxy::output::{OutputSender, SinkOutput};
use serde_json::Value;

#[derive(Debug, Default)]
pub struct Stdout {}

impl Stdout {
    pub fn new() -> Self {
        Self {}
    }

    pub fn builder() -> StdoutBuilder {
        StdoutBuilder {}
    }
}

impl ChannelSink for Stdout {
    fn send(&mut self, data: Value) -> Result<Vec<SinkOutput>, Box<dyn std::error::Error>> {
        let data = match data {
            Value::String(s) => s,
            other => other.to_string(),
        };
        println!("{data}");

        Ok(vec![])
    }
}

#[derive(Default)]
pub struct StdoutBuilder {}

impl ChannelSinkBuilder for StdoutBuilder {
    fn build(
        &self,
        _channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn ChannelSink>, Vec<BuildError>> {
        Ok(Box::new(Stdout::new()))
    }
}
