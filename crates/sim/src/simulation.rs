use crate::Signal;
use crate::build::{BuildError, SimulationKey};
use crate::channel::Channel;
use crate::effect::{Effect, EffectBuilder, EffectEvent};
use crate::log::{Log, SimpleEventLog};
use crate::util::time::Moment;
use chrono::{TimeDelta, Utc};
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub struct Simulation {
    event_log: Box<dyn Log>,
    effects: Vec<Effect>,
    channels: Vec<Channel>,
    signal_tx: Sender<Signal>,
    signal_rx: Receiver<Signal>,
    limit: Option<u64>,
    emitted: u64,
}

impl Simulation {
    pub fn builder() -> SimulationBuilder {
        SimulationBuilder::new()
    }

    pub fn signal_tx(&self) -> Sender<Signal> {
        self.signal_tx.clone()
    }

    /// Hands ownership of the simulation's channels to the caller (e.g. the CLI's channel
    /// dispatch), leaving this simulation's copy empty.
    pub fn take_channels(&mut self) -> Vec<Channel> {
        std::mem::take(&mut self.channels)
    }

    /// Pushes any signals currently waiting in the channel into the event log.
    fn drain_signals(&mut self) {
        for signal in self.signal_rx.try_iter() {
            self.event_log.push(signal.into());
        }
    }

    /// Finalizes the simulation once effect dispatch has fully shut down.
    ///
    /// Iteration already drains signals before computing each event, but signals sent after
    /// the last event (e.g. a `stream` channel's subprocess flushing its output once it
    /// receives EOF) arrive after the last `next()` call, so this drains once more. Taking
    /// `self` by value also ensures the event log is dropped - and so commits any pending
    /// writes - before this returns.
    pub fn finish(mut self) {
        self.drain_signals();
    }
}

impl Iterator for Simulation {
    type Item = EffectEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.drain_signals();

        if self.limit.is_some_and(|limit| self.emitted >= limit) {
            return None;
        }

        loop {
            self.effects
                .sort_unstable_by_key(|e| e.next_offset().unwrap_or(u64::MAX));

            match self.effects.first_mut()?.next()? {
                Ok(effect_event) => {
                    self.emitted += 1;
                    self.event_log.push(effect_event.clone().into());
                    return Some(effect_event);
                }
                Err(skipped_event) => {
                    self.emitted += 1;
                    self.event_log.push(skipped_event.into());
                    if self.limit.is_some_and(|limit| self.emitted >= limit) {
                        return None;
                    }
                    continue;
                }
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
    channels: Vec<Channel>,
    limit: Option<u64>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            seed: 1,
            start: Moment::Relative(TimeDelta::days(-30)),
            end: Moment::Relative(TimeDelta::zero()),
            event_log: Box::new(SimpleEventLog::default()),
            effect_builders: vec![],
            channels: vec![],
            limit: None,
        }
    }

    pub fn log(mut self, log: impl Log + 'static) -> Self {
        self.event_log = Box::new(log);
        self
    }

    /// Caps the total number of events (effects and errors combined) the built [`Simulation`]
    /// will emit before its iterator ends.
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.set_seed(seed);
        self
    }

    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = seed;
        self
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = start;
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = end;
        self
    }

    pub fn set_effect(&mut self, effect: EffectBuilder) {
        self.effect_builders.push(effect)
    }

    pub fn set_channel(&mut self, channel: Channel) -> &mut Self {
        self.channels.push(channel);
        self
    }

    pub fn with_effect(
        &mut self,
        key: &str,
        f: impl FnOnce(EffectBuilder) -> EffectBuilder,
    ) -> &mut Self {
        let builder = Effect::builder(key.into());
        let builder = f(builder);
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
            effect_builder
                .set_now(now)
                .set_sim_start(start)
                .set_sim_end(end)
                .set_event_log(self.event_log.reader())
                .set_seed(self.seed);

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
                channels: self.channels,
                signal_tx,
                signal_rx,
                limit: self.limit,
                emitted: 0,
            })
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::build::BuildError;
    use crate::schema::{
        Metadata, Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult,
    };

    /// A schema that deterministically alternates between succeeding and failing on
    /// every other call, so a test can know exactly how many `Ok`s and `Err`s a fixed
    /// number of calls produces without depending on any effect's trigger timing.
    #[derive(Debug, Default)]
    struct AlternatingSchema {
        calls: u32,
    }

    impl Schema for AlternatingSchema {
        fn next(&mut self, _context: &SchemaContext) -> SchemaResult {
            self.calls += 1;
            if self.calls % 2 == 1 {
                SchemaResult {
                    value: Some(serde_json::Value::Null),
                    metadata: vec![],
                }
            } else {
                SchemaResult {
                    value: None,
                    metadata: vec![Metadata {
                        mtype: "error".into(),
                        attribute: None,
                        value: Some(serde_json::json!({ "message": "boom" })),
                    }],
                }
            }
        }
    }

    #[derive(Debug)]
    struct AlternatingSchemaBuilder;

    impl SchemaBuilder for AlternatingSchemaBuilder {
        fn build(&self, _visitor: SchemaBuildVisitor) -> Result<Box<dyn Schema>, Vec<BuildError>> {
            Ok(Box::new(AlternatingSchema::default()))
        }
    }

    #[test]
    fn limit_counts_effects_and_errors_together() {
        let mut simulation_builder = super::Simulation::builder();

        simulation_builder.with_effect("alternating", |e| {
            e.trigger_hertz(1000.0).schema(AlternatingSchemaBuilder)
        });

        let events: Vec<_> = simulation_builder.limit(5).build().unwrap().collect();

        // 5 emitted total (limit), alternating Ok, Err, Ok, Err, Ok - so 3 are yielded.
        assert_eq!(
            events.len(),
            3,
            "limit should count both effect and error events toward the cap"
        );
    }
}
