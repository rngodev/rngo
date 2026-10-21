pub mod sink;

use crate::BuildError;
use crate::proxy::format::Format;
use crate::proxy::output::{Output, OutputSender, SinkOutput};
use serde_json::Value;
use std::error::Error;
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct Channel {
    pub(super) key: String,
    pub(super) format: Option<Box<dyn Format>>,
    pub(super) sink: Box<dyn ChannelSink>,
    pub(super) effects: Vec<String>,
}

impl Channel {
    pub fn builder(key: String) -> ChannelBuilder {
        ChannelBuilder::new(key)
    }
}

pub trait ChannelSink: std::fmt::Debug {
    /// Sends one event's `data`: the channel's formatted data as a string, or the input's own
    /// `data` when the channel has no format. The proxy ties returned outputs to the input.
    fn send(&mut self, data: Value) -> Result<Vec<SinkOutput>, Box<dyn Error>>;
    fn finish(&mut self) {}
}

/// Built with the channel's own key passed in at build time, rather than baked in at
/// construction - lets a sink builder (e.g. [`crate::build::exec`]) be constructed generically,
/// before the channel it'll belong to is known, mirroring how `outputs` is threaded in.
pub trait ChannelSinkBuilder {
    fn build(
        &self,
        channel_key: &str,
        outputs: OutputSender,
    ) -> Result<Box<dyn ChannelSink>, Vec<BuildError>>;
}

pub struct ChannelBuilder {
    key: String,
    format: Option<Box<dyn Format>>,
    channel_sink_builder: Option<Box<dyn ChannelSinkBuilder>>,
    output_tx: Option<Sender<Output>>,
    effects: Vec<String>,
}

impl ChannelBuilder {
    fn new(key: String) -> Self {
        ChannelBuilder {
            key,
            format: None,
            channel_sink_builder: None,
            output_tx: None,
            effects: vec![],
        }
    }

    pub fn format(mut self, format: impl Format + 'static) -> Self {
        self.set_format(Box::new(format));
        self
    }

    pub fn set_format(&mut self, format: Box<dyn Format>) -> &mut Self {
        self.format = Some(format);
        self
    }

    pub fn sink(mut self, builder: impl ChannelSinkBuilder + 'static) -> Self {
        self.set_sink(Box::new(builder));
        self
    }

    pub fn set_sink(&mut self, builder: Box<dyn ChannelSinkBuilder>) -> &mut Self {
        self.channel_sink_builder = Some(builder);
        self
    }

    pub fn output_tx(mut self, output_tx: Sender<Output>) -> Self {
        self.set_output_tx(output_tx);
        self
    }

    pub fn set_output_tx(&mut self, output_tx: Sender<Output>) -> &mut Self {
        self.output_tx = Some(output_tx);
        self
    }

    pub fn effects(mut self, effects: Vec<String>) -> Self {
        self.set_effects(effects);
        self
    }

    pub fn set_effects(&mut self, effects: Vec<String>) -> &mut Self {
        self.effects = effects;
        self
    }

    pub fn build(self) -> Result<Channel, Vec<BuildError>> {
        let sink = if let Some(sink_builder) = self.channel_sink_builder {
            if let Some(output_tx) = self.output_tx {
                sink_builder.build(&self.key, OutputSender::new(&self.key, output_tx))
            } else {
                Err(vec![BuildError::Channel {
                    channel: self.key.clone(),
                    message: "output_tx was not set".into(),
                }])
            }
        } else {
            Err(vec![BuildError::Channel {
                channel: self.key.clone(),
                message: "sink was not set".into(),
            }])
        }?;

        Ok(Channel {
            key: self.key,
            format: self.format,
            sink,
            effects: self.effects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;

    /// Records whatever `channel_key` it's built with, so a test can confirm `ChannelBuilder`
    /// threads its own key through to a sink builder constructed with no key of its own - the
    /// case a Rust-DSL builder like `exec()`/`stream()` relies on.
    #[derive(Debug)]
    struct RecordingSink;

    impl ChannelSink for RecordingSink {
        fn send(&mut self, _data: Value) -> Result<Vec<SinkOutput>, Box<dyn Error>> {
            Ok(vec![])
        }
    }

    #[derive(Debug)]
    struct RecordingSinkBuilder(Rc<RefCell<Option<String>>>);

    impl ChannelSinkBuilder for RecordingSinkBuilder {
        fn build(
            &self,
            channel_key: &str,
            _outputs: OutputSender,
        ) -> Result<Box<dyn ChannelSink>, Vec<BuildError>> {
            *self.0.borrow_mut() = Some(channel_key.to_string());
            Ok(Box::new(RecordingSink))
        }
    }

    #[test]
    fn build_passes_the_channel_own_key_to_the_sink_builder() {
        let seen = Rc::new(RefCell::new(None));
        let (output_tx, _output_rx) = mpsc::channel();

        Channel::builder("db".into())
            .sink(RecordingSinkBuilder(seen.clone()))
            .output_tx(output_tx)
            .build()
            .unwrap();

        assert_eq!(seen.borrow().as_deref(), Some("db"));
    }
}
