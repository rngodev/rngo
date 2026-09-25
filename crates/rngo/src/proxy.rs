pub mod channel;
pub mod format;
pub mod output;

use crate::{BuildError, Input, RunLogWriter, SimpleEventRunLog};
use channel::target::Stdout;
use channel::{Channel, ChannelBuilder};
use output::Output;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver};

pub struct Proxy {
    run_log_writer: Rc<dyn RunLogWriter>,
    channels: HashMap<String, Channel>,
    effect_channels: HashMap<String, String>,
    output_rx: Receiver<Output>,
}

impl Proxy {
    pub fn builder() -> ProxyBuilder {
        ProxyBuilder::new()
    }

    pub fn send(&mut self, input: &Input) -> Result<(), Box<dyn std::error::Error>> {
        let channel_key = match self.effect_channels.get(&input.effect) {
            Some(k) => k,
            None => return Ok(()),
        };

        if let Some(channel) = self.channels.get_mut(channel_key) {
            let formatted_data = if let Some(format) = &channel.format {
                format.format(input).ok()
            } else {
                None
            };

            let outputs = channel.target.send(input, formatted_data)?;
            for output in outputs {
                self.run_log_writer.push_output(output);
            }
        };

        self.drain_outputs();
        Ok(())
    }

    pub fn finish(&mut self) {
        for channel in self.channels.values_mut() {
            channel.target.finish();
        }
        self.drain_outputs();
    }

    fn drain_outputs(&mut self) {
        while let Ok(output) = self.output_rx.try_recv() {
            self.run_log_writer.push_output(output);
        }
    }
}

pub struct ProxyBuilder {
    run_log_writer: Option<Rc<dyn RunLogWriter>>,
    channel_builders: Vec<ChannelBuilder>,
    stdout: bool,
}

impl ProxyBuilder {
    pub fn new() -> Self {
        Self {
            run_log_writer: None,
            channel_builders: vec![],
            stdout: false,
        }
    }

    pub fn run_log_writer<T: RunLogWriter + 'static>(mut self, writer: Rc<T>) -> Self {
        self.run_log_writer = Some(writer as Rc<dyn RunLogWriter>);
        self
    }

    pub fn stdout(mut self, value: bool) -> Self {
        self.set_stdout(value);
        self
    }

    pub fn set_stdout(&mut self, value: bool) -> &mut Self {
        self.stdout = value;
        self
    }

    pub fn set_channel(&mut self, channel: ChannelBuilder) {
        self.channel_builders.push(channel)
    }

    pub fn with_channel(
        &mut self,
        key: &str,
        f: impl FnOnce(ChannelBuilder) -> ChannelBuilder,
    ) -> &mut Self {
        let builder = Channel::builder(key.into());
        let builder = f(builder);
        self.channel_builders.push(builder);
        self
    }

    pub fn build(self) -> Result<Proxy, Vec<BuildError>> {
        let mut errors = vec![];
        let mut channels = HashMap::new();
        let (output_tx, output_rx) = mpsc::channel::<Output>();

        let run_log_writer = self
            .run_log_writer
            .unwrap_or_else(|| SimpleEventRunLog::new());

        for mut channel_builder in self.channel_builders {
            if self.stdout {
                channel_builder.set_target(Box::new(Stdout::builder()));
            }
            channel_builder.set_output_tx(output_tx.clone());

            match channel_builder.build() {
                Ok(channel) => {
                    channels.insert(channel.key.clone(), channel);
                }
                Err(mut e) => errors.append(&mut e),
            };
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        let effect_channels = channels
            .values()
            .flat_map(|channel| {
                channel
                    .effects
                    .iter()
                    .map(move |effect_key| (effect_key.clone(), channel.key.clone()))
            })
            .collect();

        Ok(Proxy {
            run_log_writer,
            channels,
            effect_channels,
            output_rx,
        })
    }
}

impl Default for ProxyBuilder {
    fn default() -> Self {
        Self::new()
    }
}
