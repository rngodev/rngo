use crate::channel::ChannelBuilder;
use crate::{BuildError, Channel, Input, Output};
use std::collections::HashMap;
use std::sync::mpsc::Sender;

pub struct System {
    pub channels: HashMap<String, Channel>,
    pub effect_channels: HashMap<String, String>,
}

impl System {
    pub fn builder() -> SystemBuilder {
        SystemBuilder::new()
    }

    pub fn send(&mut self, input: &Input) -> Result<(), Box<dyn std::error::Error>> {
        let channel_key = match self.effect_channels.get(&input.effect) {
            Some(k) => k.clone(),
            None => return Ok(()),
        };

        match self.channels.get_mut(&channel_key) {
            Some(channel) => {
                let formatted_data = if let Some(format) = &channel.format {
                    format.format(input).ok()
                } else {
                    None
                };

                channel.target.send(input, formatted_data)
            }
            None => Ok(()),
        }
    }

    pub fn finish(self) {
        drop(self.channels);
    }
}

pub struct SystemBuilder {
    output_tx: Option<Sender<Output>>,
    channel_builders: Vec<ChannelBuilder>,
}

impl SystemBuilder {
    pub fn new() -> Self {
        Self {
            output_tx: None,
            channel_builders: vec![],
        }
    }

    pub fn output_tx(mut self, output_tx: Sender<Output>) -> Self {
        self.set_output_tx(output_tx);
        self
    }

    pub fn set_output_tx(&mut self, output_tx: Sender<Output>) -> &mut Self {
        self.output_tx = Some(output_tx);
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

    pub fn build(self) -> Result<System, Vec<BuildError>> {
        let mut errors = vec![];
        let mut channels = HashMap::new();

        if let Some(output_tx) = self.output_tx {
            for mut channel_builder in self.channel_builders {
                channel_builder.set_output_tx(output_tx.clone());

                match channel_builder.build() {
                    Ok(channel) => {
                        channels.insert(channel.key.clone(), channel);
                    }
                    Err(mut e) => errors.append(&mut e),
                };
            }
        } else {
            errors.push(BuildError::System {
                message: "output_tx was not set".into(),
            });
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

        Ok(System {
            channels,
            effect_channels,
        })
    }
}
