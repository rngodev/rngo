mod exec;
mod stream;

use crate::format::Format;
use crate::{BuildError, Input, Output, spec};
use std::error::Error;
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct Channel {
    pub key: String,
    pub format: Option<Box<dyn Format>>,
    pub target: spec::ChannelTarget,
}

pub trait ChannelTarget {
    fn send(&mut self, input: &Input, data: Option<String>) -> Result<(), Box<dyn Error>>;
}

pub trait ChannelTargetBuilder {
    fn build(self, output_tx: Sender<Output>) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>>;
}
