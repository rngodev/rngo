use crate::build::{BuildError, SimulationKey};
use crate::effect::{Effect, EffectBuilder};
use crate::event::{Event, EventLog, SimpleEventLog};
use crate::util::time::Moment;
use chrono::{TimeDelta, Utc};
use std::rc::Rc;

#[derive(Debug)]
pub struct Simulation {
    event_log: Rc<dyn EventLog>,
    effects: Vec<Effect>,
}

impl Simulation {
    pub fn builder() -> SimulationBuilder {
        SimulationBuilder::new()
    }
}

impl Iterator for Simulation {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        self.effects
            .sort_unstable_by_key(|e| e.next_offset().unwrap_or(u64::MAX));

        if let Some(event) = self.effects.first_mut()?.next() {
            self.event_log.push(&event);
            Some(event)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct SimulationBuilder {
    pub seed: u64,
    pub start: Moment,
    pub end: Moment,
    event_log: Rc<dyn EventLog>,
    effect_builders: Vec<EffectBuilder>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            seed: 1,
            start: Moment::Relative(TimeDelta::days(-30)),
            end: Moment::Relative(TimeDelta::zero()),
            event_log: Rc::new(SimpleEventLog::default()),
            effect_builders: vec![],
        }
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

        for effect_builder in self.effect_builders {
            match effect_builder.build(&self.event_log, self.seed) {
                Ok(effect) => effects.push(effect),
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            Ok(Simulation {
                event_log: self.event_log,
                effects,
            })
        } else {
            Err(errors)
        }
    }
}
