use indexmap::IndexMap;

use crate::channel::{ChannelBuilder, Stdout};
use crate::signal::Signal;
use crate::simulation::SimulationBuilder;
use crate::{
    BuildError, Channel, EffectMetadata, Input, Output, RunLog, SignalOutcome, SimpleEventRunLog,
    SimulationEvent,
};
use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

pub struct System {
    run_log: Box<dyn RunLog>,
    channels: HashMap<String, Channel>,
    effect_channels: HashMap<String, String>,
    signals: IndexMap<String, Box<dyn Signal>>,
    output_rx: Receiver<Output>,
}

impl System {
    pub fn builder() -> SystemBuilder {
        SystemBuilder::new()
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
            self.run_log.push_input(input.clone());
            for output in outputs {
                self.run_log.push_output(output);
            }
        };

        self.drain_outputs();
        Ok(())
    }

    pub fn add_metadata(&mut self, metadata: EffectMetadata) {
        self.run_log.push_metadata(metadata);
    }

    pub fn run(
        &mut self,
        simulation_builder: SimulationBuilder,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut simulation = simulation_builder
            .run_log_reader(self.run_log.reader())
            .build()
            .unwrap(); // TODO: FIX

        for event in &mut simulation {
            match event {
                SimulationEvent::Input(input) => {
                    self.send(&input)?;
                }
                SimulationEvent::SkippedInput(skipped_input) => {
                    self.add_metadata(skipped_input.into());
                }
            }
        }

        Ok(())
    }

    pub fn audit(&self) -> IndexMap<String, SignalOutcome> {
        self.signals
            .iter()
            .map(|(key, signal)| (key.clone(), signal.evaluate(self.run_log.as_ref())))
            .collect()
    }

    /// Shuts down every channel's target (e.g. closing a `stream` subprocess's stdin and
    /// waiting for it to exit). This can itself produce trailing outputs, so `System` remains
    /// iterable afterward - drain it before dropping to pick those up.
    pub fn finish(&mut self) {
        self.channels.clear();
        self.drain_outputs();
    }

    fn drain_outputs(&mut self) {
        while let Some(output) = self.output_rx.try_recv().ok() {
            self.run_log.push_output(output);
        }
    }
}

pub struct SystemBuilder {
    run_log: Option<Box<dyn RunLog>>,
    channel_builders: Vec<ChannelBuilder>,
    signals: IndexMap<String, Box<dyn Signal>>,
    stdout: bool,
}

impl SystemBuilder {
    pub fn new() -> Self {
        Self {
            run_log: None,
            channel_builders: vec![],
            signals: IndexMap::new(),
            stdout: false,
        }
    }

    pub fn run_log(mut self, run_log: impl RunLog + 'static) -> Self {
        self.run_log = Some(Box::new(run_log));
        self
    }

    /// Routes every channel's events to stdout instead of its configured target (e.g. a `stream`
    /// subprocess never gets spawned). Each channel's `format`, if any, still applies, so stdout
    /// output looks like what the channel would have actually sent.
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

    pub fn signals(mut self, signals: IndexMap<String, Box<dyn Signal>>) -> Self {
        self.set_signals(signals);
        self
    }

    pub fn set_signals(&mut self, signals: IndexMap<String, Box<dyn Signal>>) -> &mut Self {
        self.signals = signals;
        self
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
        let (output_tx, output_rx) = mpsc::channel::<Output>();

        let run_log = self
            .run_log
            .unwrap_or_else(|| Box::new(SimpleEventRunLog::new(12345)));

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

        Ok(System {
            run_log,
            channels,
            effect_channels,
            signals: self.signals,
            output_rx,
        })
    }
}

impl Default for SystemBuilder {
    fn default() -> Self {
        Self::new()
    }
}
