use crate::format::Format;
use crate::spec::ChannelTarget;

#[derive(Debug)]
pub struct Channel {
    pub key: String,
    pub format: Option<Box<dyn Format>>,
    pub target: ChannelTarget,
}
