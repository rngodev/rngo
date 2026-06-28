use crate::Signal;
use crate::build::{BuildError, SimulationKey};
use crate::effect::{Effect, EffectBuilder, EffectEvent};
use crate::log::{Log, SimpleEventLog};
use crate::util::time::Moment;
use chrono::{TimeDelta, Utc};
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub struct Simulation {
    event_log: Box<dyn Log>,
    effects: Vec<Effect>,
    signal_tx: Sender<Signal>,
    signal_rx: Receiver<Signal>,
}

impl Simulation {
    pub fn builder() -> SimulationBuilder {
        SimulationBuilder::new()
    }

    pub fn signal_tx(&self) -> Sender<Signal> {
        self.signal_tx.clone()
    }
}

impl Iterator for Simulation {
    type Item = EffectEvent;

    fn next(&mut self) -> Option<Self::Item> {
        for signal in self.signal_rx.try_iter() {
            self.event_log.push(signal.into());
        }

        loop {
            self.effects
                .sort_unstable_by_key(|e| e.next_offset().unwrap_or(u64::MAX));

            match self.effects.first_mut()?.next() {
                Some(Ok(effect_event)) => {
                    self.event_log.push(effect_event.clone().into());
                    return Some(effect_event);
                }
                Some(Err(error)) => {
                    self.event_log.push(error.into());
                    continue;
                }
                None => return None,
            }
        }
    }
}

#[derive(Debug)]
pub struct SimulationBuilder {
    pub seed: u64,
    pub start: Moment,
    pub end: Moment,
    event_log: Box<dyn Log>,
    effect_builders: Vec<EffectBuilder>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            seed: 1,
            start: Moment::Relative(TimeDelta::days(-30)),
            end: Moment::Relative(TimeDelta::zero()),
            event_log: Box::new(SimpleEventLog::default()),
            effect_builders: vec![],
        }
    }

    pub fn log(mut self, log: impl Log + 'static) -> Self {
        self.event_log = Box::new(log);
        self
    }

    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = seed;
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = start;
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = end;
        self
    }

    pub fn add_effect(&mut self, effect: EffectBuilder) {
        self.effect_builders.push(effect)
    }

    pub fn with_effect(&mut self, key: &str, f: impl FnOnce(&mut EffectBuilder)) -> &mut Self {
        let mut builder = Effect::builder(key.into());
        f(&mut builder);
        self.effect_builders.push(builder);
        self
    }

    pub fn build(self) -> Result<Simulation, Vec<BuildError>> {
        let mut errors = vec![];
        let now = Utc::now().fixed_offset();
        let start = self.start.resolve(now);
        let end = self.end.resolve(now);

        if start >= end {
            errors.push(BuildError::Simulation {
                key: SimulationKey::Start,
                message: "start must be before end".into(),
            });
        }

        let mut effects = vec![];

        for mut effect_builder in self.effect_builders {
            effect_builder.set_now(now);
            effect_builder.set_sim_start(start);
            effect_builder.set_sim_end(end);
            effect_builder.set_event_log(self.event_log.reader());
            effect_builder.set_seed(self.seed);
            match effect_builder.build() {
                Ok(effect) => effects.push(effect),
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            let (signal_tx, signal_rx) = mpsc::channel::<Signal>();
            Ok(Simulation {
                event_log: self.event_log,
                effects,
                signal_tx,
                signal_rx,
            })
        } else {
            Err(errors)
        }
    }
}
