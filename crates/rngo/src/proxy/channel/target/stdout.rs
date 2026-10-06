use crate::BuildError;
use crate::proxy::channel::{ChannelTarget, ChannelTargetBuilder};
use crate::proxy::output::{OutputSender, TargetOutput};
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

impl ChannelTarget for Stdout {
    fn send(&mut self, data: Value) -> Result<Vec<TargetOutput>, Box<dyn std::error::Error>> {
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

impl ChannelTargetBuilder for StdoutBuilder {
    fn build(
        &self,
        _channel_key: &str,
        _outputs: OutputSender,
    ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
        Ok(Box::new(Stdout::new()))
    }
}
