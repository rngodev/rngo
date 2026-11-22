use crate::effect::generator::GeneratorMetadata;
use jiff::Timestamp;
use serde::Serialize;
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Level {
    Error,
    Warning,
    Info,
}

/// Something a channel reported, as written to the run log.
#[derive(Clone, Debug, Serialize)]
#[non_exhaustive]
pub struct Output {
    pub input_id: Option<u64>,
    pub timestamp: Timestamp,
    pub channel: String,
    pub level: Level,
    pub data: String,
    pub metadata: Vec<GeneratorMetadata>,
}

impl Output {
    /// An output from `channel`, timestamped now and tied to no input.
    pub fn new(channel: impl Into<String>, level: Level, data: impl Into<String>) -> Self {
        Output {
            input_id: None,
            timestamp: Timestamp::now(),
            channel: channel.into(),
            level,
            data: data.into(),
            metadata: vec![],
        }
    }

    /// Ties it to the input it was reported for.
    pub fn input_id(mut self, input_id: u64) -> Self {
        self.input_id = Some(input_id);
        self
    }

    pub fn timestamp(mut self, timestamp: Timestamp) -> Self {
        self.timestamp = timestamp;
        self
    }

    pub fn metadata(mut self, metadata: Vec<GeneratorMetadata>) -> Self {
        self.metadata = metadata;
        self
    }
}

/// A message a channel reports; the proxy turns it into an [`Output`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ChannelOutput {
    pub level: Level,
    pub message: String,
}

impl ChannelOutput {
    pub fn new(level: Level, message: impl Into<String>) -> Self {
        ChannelOutput {
            level,
            message: message.into(),
        }
    }

    /// Converts to an [`Output`] on `channel`, timestamped now.
    pub(crate) fn into_output(self, channel: &str, input_id: Option<u64>) -> Output {
        let output = Output::new(channel, self.level, self.message);
        match input_id {
            Some(input_id) => output.input_id(input_id),
            None => output,
        }
    }
}

/// Lets a channel report outputs outside of `send` (e.g. from a reader thread); each is
/// timestamped when sent and tied to no input.
#[derive(Clone, Debug)]
pub struct OutputSender {
    channel: String,
    tx: Sender<Output>,
}

impl OutputSender {
    pub(crate) fn new(channel: impl Into<String>, tx: Sender<Output>) -> Self {
        OutputSender {
            channel: channel.into(),
            tx,
        }
    }

    pub fn send(&self, output: ChannelOutput) {
        let _ = self.tx.send(output.into_output(&self.channel, None));
    }
}
