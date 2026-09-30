use crate::effect::schema::Metadata;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, Serialize)]
pub struct Output {
    pub input_id: Option<u64>,
    pub timestamp: DateTime<Utc>,
    pub channel: String,
    pub level: Level,
    pub data: String,
    pub metadata: Vec<Metadata>,
}

/// A message a channel target reports; the proxy turns it into an [`Output`].
#[derive(Clone, Debug, PartialEq)]
pub struct TargetOutput {
    pub level: Level,
    pub message: String,
}

impl TargetOutput {
    pub fn new(level: Level, message: impl Into<String>) -> Self {
        TargetOutput {
            level,
            message: message.into(),
        }
    }

    /// Converts to an [`Output`] on `channel`, timestamped now.
    pub fn into_output(self, channel: &str, input_id: Option<u64>) -> Output {
        Output {
            input_id,
            timestamp: Utc::now(),
            channel: channel.to_string(),
            level: self.level,
            data: self.message,
            metadata: vec![],
        }
    }
}

/// Lets a channel target report outputs outside of `send` (e.g. from a reader thread); each is
/// timestamped when sent and tied to no input.
#[derive(Clone, Debug)]
pub struct OutputSender {
    channel: String,
    tx: Sender<Output>,
}

impl OutputSender {
    pub fn new(channel: impl Into<String>, tx: Sender<Output>) -> Self {
        OutputSender {
            channel: channel.into(),
            tx,
        }
    }

    pub fn send(&self, output: TargetOutput) {
        let _ = self.tx.send(output.into_output(&self.channel, None));
    }
}
