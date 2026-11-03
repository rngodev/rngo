mod clock;
pub mod generator;
pub mod phase;
pub mod simulation;
mod trigger;

use crate::build::{BuildError, EffectKey};
use crate::effect::phase::{Phase, PhaseBuilder};
use crate::log::{Metadata, RunLogReader, SimpleEventRunLog};
use crate::moment::Moment;
use chrono::{DateTime, FixedOffset, TimeDelta};
use clock::Clock;
use generator::{
    Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext,
    Metadata as GeneratorMetadata,
};
use multi_try::MultiTry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::num::NonZeroU64;
use std::rc::Rc;
use trigger::{Trigger, TriggerConfig};

pub use trigger::TriggerEvent;

#[derive(Debug)]
pub struct Effect {
    pub key: String,
    run_log_reader: Rc<dyn RunLogReader>,
    trigger: Trigger,
    generator: Box<dyn Generator>,
    end_offset: Option<i64>,
    limit: Option<u64>,
    produced: u64,
    origin: DateTime<FixedOffset>,
    pub start: DateTime<FixedOffset>,
    pub end: Option<DateTime<FixedOffset>>,
    /// The name of the phase the effect asked for; `None` for the default phase.
    pub phase: Option<String>,
    simulation_start: DateTime<FixedOffset>,
    simulation_end: DateTime<FixedOffset>,
}

/// What an effect's window comes to once its own values are laid over its phase's: a start, and an end, a limit or both.
#[derive(Clone, Copy, Debug)]
struct Window {
    pub start: DateTime<FixedOffset>,
    pub end: Option<DateTime<FixedOffset>>,
    pub limit: Option<u64>,
}

impl Effect {
    pub fn builder(key: String) -> EffectBuilder {
        EffectBuilder::new(key)
    }

    /// Milliseconds since the simulation origin at which this effect next fires.
    pub fn next_offset(&self) -> Option<i64> {
        if self.limit.is_some_and(|limit| self.produced >= limit) {
            return None;
        }

        let offset = self.trigger.next_offset()?;
        if self
            .end_offset
            .is_some_and(|end_offset| offset > end_offset)
        {
            None
        } else {
            Some(offset)
        }
    }
}

impl Iterator for Effect {
    type Item = Result<Input, SkippedInput>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_offset()?;

        let trigger_event = self.trigger.pull()?;
        let context = GeneratorContext {
            trigger: &trigger_event,
            simulation_start: self.simulation_start,
            simulation_end: self.simulation_end,
        };
        let timestamp = self.origin + TimeDelta::milliseconds(trigger_event.sim_offset);

        let result = self.generator.next(&context);

        if let Some(data) = result.value {
            let last_id = self.run_log_reader.last().map(|e| e.id).unwrap_or(0);
            self.produced += 1;

            Some(Ok(Input {
                id: last_id + 1,
                effect: self.key.clone(),
                timestamp,
                data,
                metadata: result.metadata,
            }))
        } else {
            Some(Err(SkippedInput {
                effect: self.key.clone(),
                timestamp,
                metadata: result.metadata,
            }))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    pub id: u64,
    pub effect: String,
    pub timestamp: DateTime<FixedOffset>,
    pub data: Value,
    pub metadata: Vec<GeneratorMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedInput {
    pub effect: String,
    pub timestamp: DateTime<FixedOffset>,
    pub metadata: Vec<GeneratorMetadata>,
}

impl From<SkippedInput> for Metadata {
    fn from(skipped: SkippedInput) -> Self {
        let SkippedInput {
            effect,
            timestamp,
            metadata,
        } = skipped;

        Metadata {
            mtype: "skipped".to_string(),
            input_id: None,
            output_id: None,
            data: Some(serde_json::json!({ "effect": effect, "metadata": metadata })),
            segment: None,
            timestamp: Some(timestamp),
        }
    }
}

#[derive(Debug)]
pub struct EffectBuilder {
    key: String,
    custom_phase: Option<PhaseBuilder>,
    phase_key: Option<String>,
    phase: Option<Phase>,
    seed: Option<u64>,
    origin: Option<DateTime<FixedOffset>>,
    simulation: Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)>,
    event_run_log: Option<Rc<dyn RunLogReader>>,
    trigger: TriggerConfig,
    generator_builder: Option<Box<dyn GeneratorBuilder>>,
}

impl EffectBuilder {
    fn new(key: String) -> Self {
        EffectBuilder {
            key,
            custom_phase: None,
            phase_key: None,
            phase: None,
            seed: None,
            origin: None,
            simulation: None,
            event_run_log: None,
            trigger: TriggerConfig::ClockExpression("hz(1, day)".into()),
            generator_builder: None,
        }
    }

    fn custom_phase_mut(&mut self) -> &mut PhaseBuilder {
        self.custom_phase.get_or_insert_with(PhaseBuilder::default)
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.custom_phase_mut().set_start(start);
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.custom_phase_mut().set_end(end);
        self
    }

    /// Sets `start` from an expression, parsed when the effect builds so it can refer to the default phase's bounds via `simulation`; overrides `start`.
    pub fn set_start_expression(&mut self, expression: String) -> &mut Self {
        self.custom_phase_mut().set_start_expression(expression);
        self
    }

    /// Sets `end` from an expression, parsed when the effect builds so it can refer to the default phase's bounds via `simulation`; overrides `end`.
    pub fn set_end_expression(&mut self, expression: String) -> &mut Self {
        self.custom_phase_mut().set_end_expression(expression);
        self
    }

    pub fn limit(mut self, limit: NonZeroU64) -> Self {
        self.set_limit(limit);
        self
    }

    pub fn set_limit(&mut self, limit: NonZeroU64) -> &mut Self {
        self.custom_phase_mut().set_limit(limit);
        self
    }

    pub fn phase_key(mut self, key: impl Into<String>) -> Self {
        self.set_phase_key(key);
        self
    }

    /// Sets the key of the phase this effect belongs to; no key means the default phase and a key matching no phase an empty one.
    pub fn set_phase_key(&mut self, key: impl Into<String>) -> &mut Self {
        self.phase_key = Some(key.into());
        self
    }

    pub fn phase(mut self, phase: &Phase) -> Self {
        self.set_phase(phase);
        self
    }

    /// Sets the phase whose `start`, `end` and `limit` the effect takes where it sets none itself; without one the phase is empty.
    pub fn set_phase(&mut self, phase: &Phase) -> &mut Self {
        self.phase = Some(phase.clone());
        self
    }

    pub fn now(mut self, now: DateTime<FixedOffset>) -> Self {
        self.set_now(now);
        self
    }

    /// Sets the instant relative moments resolve against; defaults to the current time at build.
    pub fn set_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.custom_phase_mut().set_now(now);
        self
    }

    /// Sets the simulation's start and end, which the effect's expressions and generators see as `simulation`; default to the effect's own start and end.
    pub fn set_simulation(
        &mut self,
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
    ) -> &mut Self {
        self.simulation = Some((start, end));
        self.custom_phase_mut().set_simulation(start, end);
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.set_seed(seed);
        self
    }

    /// Sets the simulation's seed; defaults to 1.
    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = Some(seed);
        self
    }

    pub fn origin(mut self, origin: DateTime<FixedOffset>) -> Self {
        self.set_origin(origin);
        self
    }

    /// Sets the instant offsets are measured from, shared by the simulation's effects so their inputs stay comparable; defaults to the effect's own start.
    pub fn set_origin(&mut self, origin: DateTime<FixedOffset>) -> &mut Self {
        self.origin = Some(origin);
        self
    }

    /// The key of the phase this effect asked for, if any.
    pub fn get_phase_key(&self) -> Option<&str> {
        self.phase_key.as_deref()
    }

    /// The instant the effect starts at, resolved from its own `start` or its phase's; errors as `build` does for the window.
    pub fn resolved_start(&self) -> Result<DateTime<FixedOffset>, Vec<BuildError>> {
        self.window().map(|window| window.start)
    }

    pub fn event_run_log(mut self, event_run_log: Rc<dyn RunLogReader>) -> Self {
        self.set_event_run_log(event_run_log);
        self
    }

    pub fn set_event_run_log(&mut self, event_run_log: Rc<dyn RunLogReader>) -> &mut Self {
        self.event_run_log = Some(event_run_log);
        self
    }

    pub fn trigger_effect(mut self, key: String) -> Self {
        self.set_trigger_effect(key);
        self
    }

    pub fn set_trigger_effect(&mut self, key: String) -> &mut Self {
        self.trigger = TriggerConfig::Effect { key };
        self
    }

    pub fn trigger_hertz(mut self, hertz: f64) -> Self {
        self.set_trigger_hertz(hertz);
        self
    }

    pub fn set_trigger_hertz(&mut self, hertz: f64) -> &mut Self {
        self.trigger = TriggerConfig::ClockHertz(hertz);
        self
    }

    pub fn trigger_expression(mut self, expression: String) -> Self {
        self.set_trigger_expression(expression);
        self
    }

    pub fn set_trigger_expression(&mut self, expression: String) -> &mut Self {
        self.trigger = TriggerConfig::ClockExpression(expression);
        self
    }

    pub fn generator(mut self, builder: impl GeneratorBuilder + 'static) -> Self {
        self.set_generator(builder);
        self
    }

    pub fn set_generator(&mut self, builder: impl GeneratorBuilder + 'static) -> &mut Self {
        self.generator_builder = Some(Box::new(builder));
        self
    }

    fn window(&self) -> Result<Window, Vec<BuildError>> {
        let own = self
            .custom_phase
            .clone()
            .unwrap_or_default()
            .for_effect(self.key.clone())
            .build()?;
        let phase = self.phase.clone().unwrap_or_default();
        let start = own.start.or(phase.start);
        let end = own.end.or(phase.end);
        let limit = own.limit.or(phase.limit).map(NonZeroU64::get);

        let mut errors = vec![];
        if start.is_none() {
            errors.push(BuildError::effect(
                self.key.clone(),
                EffectKey::Start,
                "start must be set on the effect or its phase",
            ));
        }
        if end.is_none() && limit.is_none() {
            errors.push(BuildError::effect(
                self.key.clone(),
                EffectKey::End,
                "end or limit must be set on the effect or its phase",
            ));
        }
        if let (Some(start), Some(end)) = (start, end)
            && start >= end
        {
            errors.push(BuildError::effect(
                self.key.clone(),
                EffectKey::Start,
                "start must be before end",
            ));
        }

        match start {
            Some(start) if errors.is_empty() => Ok(Window { start, end, limit }),
            _ => Err(errors),
        }
    }

    /// Lays the effect's own `start`, `end` and `limit` over its phase's and builds it; errors unless the window has a `start` and an `end` or `limit`, and a `start` before any `end`.
    pub fn build(self) -> Result<Effect, Vec<BuildError>> {
        let window = self.window()?;
        let seed = self.seed.unwrap_or(1);
        let origin = self.origin.unwrap_or(window.start);
        let (simulation_start, simulation_end) = self
            .simulation
            .unwrap_or((window.start, window.end.unwrap_or(window.start)));
        let mut errors: Vec<BuildError> = vec![];

        let run_log_reader: Rc<dyn RunLogReader> = self
            .event_run_log
            .unwrap_or_else(|| SimpleEventRunLog::new());

        let generator_result = if let Some(generator_builder) = self.generator_builder {
            let visitor = GeneratorBuildVisitor {
                event_run_log: run_log_reader.clone(),
                simulation_seed: seed,
                effect_key: self.key.clone(),
                path: vec![],
            };

            generator_builder.build(visitor)
        } else {
            Err(vec![BuildError::effect(
                self.key.clone(),
                EffectKey::Generator,
                "generator was not set",
            )])
        };

        let trigger_result = match self.trigger {
            TriggerConfig::Effect { key } => Ok(Trigger::Effect {
                run_log_reader: run_log_reader.clone(),
                key,
                sim_start: origin,
                last_offset: 0,
            }),
            TriggerConfig::ClockHertz(hertz) => Clock::builder()
                .key(self.key.clone())
                .seed(seed)
                .hertz(hertz)
                .start_offset((window.start - origin).num_milliseconds().max(0))
                .build()
                .map(|mut clock| {
                    let next_offset = clock.next();
                    Trigger::Clock { clock, next_offset }
                }),
            TriggerConfig::ClockExpression(expression) => Clock::builder()
                .key(self.key.clone())
                .seed(seed)
                .expression(expression)
                .start_offset((window.start - origin).num_milliseconds().max(0))
                .build()
                .map(|mut clock| {
                    let next_offset = clock.next();
                    Trigger::Clock { clock, next_offset }
                }),
        };

        match generator_result
            .and_try(trigger_result)
            .map_err(|e| e.into_iter().flatten().collect::<Vec<_>>())
        {
            Ok((generator, trigger)) if errors.is_empty() => Ok(Effect {
                key: self.key,
                run_log_reader: run_log_reader.clone(),
                trigger,
                generator,
                end_offset: window
                    .end
                    .map(|end| (end - origin).num_milliseconds().max(0)),
                limit: window.limit,
                produced: 0,
                origin,
                start: window.start,
                end: window.end,
                phase: self.phase_key,
                simulation_start,
                simulation_end,
            }),
            Ok(_) => Err(errors),
            Err(mut e) => {
                errors.append(&mut e);
                Err(errors)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, EffectBuilder};
    use crate::build::{BuildError, constant};
    use crate::effect::generator::{
        Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
        Metadata,
    };
    use crate::effect::phase::Phase;
    use chrono::{TimeDelta, Utc};
    use std::num::NonZeroU64;

    #[derive(Debug, Default)]
    struct AlternatingGenerator {
        calls: u32,
    }

    impl Generator for AlternatingGenerator {
        fn next(&mut self, _context: &GeneratorContext) -> GeneratorResult {
            self.calls += 1;
            if self.calls % 2 == 1 {
                GeneratorResult {
                    value: None,
                    metadata: vec![Metadata {
                        mtype: "error".into(),
                        attribute: None,
                        data: None,
                    }],
                }
            } else {
                GeneratorResult {
                    value: Some(serde_json::Value::Null),
                    metadata: vec![],
                }
            }
        }
    }

    #[derive(Debug)]
    struct AlternatingGeneratorBuilder;

    impl GeneratorBuilder for AlternatingGeneratorBuilder {
        fn build(
            &self,
            _visitor: GeneratorBuildVisitor,
        ) -> Result<Box<dyn Generator>, Vec<BuildError>> {
            Ok(Box::new(AlternatingGenerator::default()))
        }
    }

    fn build(mut builder: EffectBuilder) -> Effect {
        let now = Utc::now().fixed_offset();
        let start = now - TimeDelta::days(30);
        builder
            .set_phase(&Phase {
                start: Some(start),
                end: Some(now),
                ..Phase::default()
            })
            .set_simulation(start, now);
        builder.build().unwrap()
    }

    #[test]
    fn limit_caps_produced_inputs() {
        let effect = build(
            Effect::builder("ping".into())
                .trigger_hertz(1000.0)
                .limit(NonZeroU64::new(3).unwrap())
                .generator(constant().value(1)),
        );

        let results: Vec<_> = effect.collect();

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(Result::is_ok));
    }

    #[test]
    fn limit_does_not_count_skipped_inputs() {
        let effect = build(
            Effect::builder("alternating".into())
                .trigger_hertz(1000.0)
                .limit(NonZeroU64::new(3).unwrap())
                .generator(AlternatingGeneratorBuilder),
        );

        let results: Vec<_> = effect.collect();

        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 3);
        assert_eq!(results.iter().filter(|r| r.is_err()).count(), 3);
    }
}
