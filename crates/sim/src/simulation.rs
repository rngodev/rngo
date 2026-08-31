use crate::build::{BuildError, SimulationKey};
use crate::effect::{Effect, EffectBuilder, Input};
use crate::run_log::{RunLog, SimpleEventRunLog};
use crate::signal::SignalOutcome;
use crate::util::time::Moment;
use crate::{Output, spec};
use chrono::{TimeDelta, Utc};
use indexmap::IndexMap;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub struct Simulation {
    event_run_log: Box<dyn RunLog>,
    effects: Vec<Effect>,
    output_tx: Sender<Output>,
    output_rx: Receiver<Output>,
    limit: Option<u64>,
    emitted: u64,
}

impl Simulation {
    pub fn builder() -> SimulationBuilder {
        SimulationBuilder::new()
    }

    pub fn output_tx(&self) -> Sender<Output> {
        self.output_tx.clone()
    }

    /// Pushes any outputs currently waiting in the channel into the run log.
    fn drain_outputs(&mut self) {
        for output in self.output_rx.try_iter() {
            self.event_run_log.push(output.into());
        }
    }

    /// Finalizes the simulation once effect dispatch has fully shut down.
    ///
    /// Iteration already drains outputs before computing each event, but outputs sent after
    /// the last event (e.g. a `stream` channel's subprocess flushing its output once it
    /// receives EOF) arrive after the last `next()` call, so this drains once more. Takes
    /// `&mut self` rather than consuming it so [`Self::evaluate_signals`] can still see these
    /// trailing outputs afterward; the run log commits its pending writes once this simulation
    /// itself is dropped.
    pub fn finish(&mut self) {
        self.drain_outputs();
    }

    /// Evaluates `signals` against everything this simulation has logged so far. Typically
    /// called after [`Self::finish`] so trailing outputs are included.
    pub fn evaluate_signals(
        &self,
        signals: &IndexMap<String, spec::Signal>,
    ) -> IndexMap<String, SignalOutcome> {
        self.event_run_log.evaluate_signals(signals)
    }
}

impl Iterator for Simulation {
    type Item = Input;

    fn next(&mut self) -> Option<Self::Item> {
        self.drain_outputs();

        if self.limit.is_some_and(|limit| self.emitted >= limit) {
            return None;
        }

        loop {
            self.effects
                .sort_unstable_by_key(|e| e.next_offset().unwrap_or(u64::MAX));

            match self.effects.first_mut()?.next()? {
                Ok(input_event) => {
                    self.emitted += 1;
                    self.event_run_log.push(input_event.clone().into());
                    return Some(input_event);
                }
                Err(skipped_event) => {
                    self.emitted += 1;
                    self.event_run_log.push(skipped_event.into());
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
    event_run_log: Option<Box<dyn RunLog>>,
    effect_builders: Vec<EffectBuilder>,
    limit: Option<u64>,
}

impl SimulationBuilder {
    fn new() -> Self {
        SimulationBuilder {
            seed: 1,
            start: Moment::Relative(TimeDelta::days(-30)),
            end: Moment::Relative(TimeDelta::zero()),
            event_run_log: None,
            effect_builders: vec![],
            limit: None,
        }
    }

    pub fn run_log(mut self, run_log: impl RunLog + 'static) -> Self {
        self.event_run_log = Some(Box::new(run_log));
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

        let event_run_log = self
            .event_run_log
            .unwrap_or_else(|| Box::new(SimpleEventRunLog::new(self.seed)));

        let mut effects = vec![];

        for mut effect_builder in self.effect_builders {
            effect_builder
                .set_now(now)
                .set_sim_start(start)
                .set_sim_end(end)
                .set_event_run_log(event_run_log.reader())
                .set_seed(self.seed);

            match effect_builder.build() {
                Ok(effect) => effects.push(effect),
                Err(mut e) => errors.append(&mut e),
            }
        }

        if errors.is_empty() {
            let (output_tx, output_rx) = mpsc::channel::<Output>();
            Ok(Simulation {
                event_run_log,
                effects,
                output_tx,
                output_rx,
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
                        data: Some(serde_json::json!({ "message": "boom" })),
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
