use crate::channel::{ChannelTarget, ChannelTargetBuilder};
use crate::{BuildError, Input, Output};
use std::sync::mpsc::Sender;

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
    fn send(
        &mut self,
        input: &Input,
        data: Option<String>,
    ) -> Result<Vec<Output>, Box<dyn std::error::Error>> {
        let data = data.unwrap_or_else(|| serde_json::to_string(&input.data).unwrap());
        println!("{data}");

        Ok(vec![])
    }
}

#[derive(Default)]
pub struct StdoutBuilder {}

impl ChannelTargetBuilder for StdoutBuilder {
    fn build(&self, _output_tx: Sender<Output>) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
        Ok(Box::new(Stdout::new()))
    }
}
