pub mod target;

use crate::format::Format;
use crate::{BuildError, Input, Output};
use std::error::Error;
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub struct Channel {
    pub key: String,
    pub format: Option<Box<dyn Format>>,
    pub target: Box<dyn ChannelTarget>,
    pub effects: Vec<String>,
}

impl Channel {
    pub fn builder(key: String) -> ChannelBuilder {
        ChannelBuilder::new(key)
    }
}

pub trait ChannelTarget: std::fmt::Debug {
    fn send(&mut self, input: &Input, data: Option<String>) -> Result<Vec<Output>, Box<dyn Error>>;
    fn finish(&mut self) {}
}

/// Built with the channel's own key passed in at build time, rather than baked in at
/// construction - lets a target builder (e.g. [`crate::build::exec`]) be constructed generically,
/// before the channel it'll belong to is known, mirroring how `output_tx` is threaded in.
pub trait ChannelTargetBuilder {
    fn build(
        &self,
        channel_key: &str,
        output_tx: Sender<Output>,
    ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>>;
}

pub struct ChannelBuilder {
    key: String,
    format: Option<Box<dyn Format>>,
    channel_target_builder: Option<Box<dyn ChannelTargetBuilder>>,
    output_tx: Option<Sender<Output>>,
    effects: Vec<String>,
}

impl ChannelBuilder {
    fn new(key: String) -> Self {
        ChannelBuilder {
            key,
            format: None,
            channel_target_builder: None,
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

    pub fn target(mut self, builder: impl ChannelTargetBuilder + 'static) -> Self {
        self.set_target(Box::new(builder));
        self
    }

    pub fn set_target(&mut self, builder: Box<dyn ChannelTargetBuilder>) -> &mut Self {
        self.channel_target_builder = Some(builder);
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
        let target = if let Some(target_builder) = self.channel_target_builder {
            if let Some(output_tx) = self.output_tx {
                target_builder.build(&self.key, output_tx)
            } else {
                Err(vec![BuildError::Channel {
                    channel: self.key.clone(),
                    message: "output_tx was not set".into(),
                }])
            }
        } else {
            Err(vec![BuildError::Channel {
                channel: self.key.clone(),
                message: "target was not set".into(),
            }])
        }?;

        Ok(Channel {
            key: self.key,
            format: self.format,
            target,
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
    /// threads its own key through to a target builder constructed with no key of its own - the
    /// case a Rust-DSL builder like `exec()`/`stream()` relies on.
    #[derive(Debug)]
    struct RecordingTarget;

    impl ChannelTarget for RecordingTarget {
        fn send(
            &mut self,
            _input: &Input,
            _data: Option<String>,
        ) -> Result<Vec<Output>, Box<dyn Error>> {
            Ok(vec![])
        }
    }

    #[derive(Debug)]
    struct RecordingTargetBuilder(Rc<RefCell<Option<String>>>);

    impl ChannelTargetBuilder for RecordingTargetBuilder {
        fn build(
            &self,
            channel_key: &str,
            _output_tx: Sender<Output>,
        ) -> Result<Box<dyn ChannelTarget>, Vec<BuildError>> {
            *self.0.borrow_mut() = Some(channel_key.to_string());
            Ok(Box::new(RecordingTarget))
        }
    }

    #[test]
    fn build_passes_the_channel_own_key_to_the_target_builder() {
        let seen = Rc::new(RefCell::new(None));
        let (output_tx, _output_rx) = mpsc::channel();

        Channel::builder("db".into())
            .target(RecordingTargetBuilder(seen.clone()))
            .output_tx(output_tx)
            .build()
            .unwrap();

        assert_eq!(seen.borrow().as_deref(), Some("db"));
    }
}
