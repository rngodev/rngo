pub mod bound;
mod clock;
pub mod generator;
pub mod prototype;
pub mod simulation;
mod trigger;

use crate::build::{BuildError, EffectKey};
use crate::effect::bound::{Bound, BoundKind};
use crate::effect::prototype::Prototype;
use crate::log::{Metadata, RunLogReader, SimpleEventRunLog};
use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
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
    trigger_config: TriggerConfig,
    seed: u64,
    generator: Box<dyn Generator>,
    end_offset: Option<i64>,
    limit: Option<u64>,
    produced: u64,
    origin: DateTime<FixedOffset>,
    pub start: DateTime<FixedOffset>,
    pub end: Option<DateTime<FixedOffset>>,
    /// The name of the prototype the effect asked for; `None` for the default prototype.
    pub prototype: Option<String>,
    simulation_start: DateTime<FixedOffset>,
    simulation_end: DateTime<FixedOffset>,
}

impl Effect {
    pub fn builder(key: String) -> EffectBuilder {
        EffectBuilder::new(key)
    }

    /// Moves the instant offsets are measured from, rebuilding the trigger around it; call before iterating, once every effect of the simulation is built and the earliest start is known.
    pub fn set_origin(
        &mut self,
        origin: DateTime<FixedOffset>,
    ) -> Result<&mut Self, Vec<BuildError>> {
        self.trigger = build_trigger(
            self.trigger_config.clone(),
            &self.key,
            self.seed,
            self.start,
            origin,
            &self.run_log_reader,
        )?;
        self.end_offset = self.end.map(|end| offset_from(end, origin));
        self.origin = origin;
        Ok(self)
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
    start: Option<Bound>,
    end: Option<Bound>,
    limit: Option<NonZeroU64>,
    now: Option<DateTime<FixedOffset>>,
    prototype_key: Option<String>,
    prototype: Option<Prototype>,
    seed: Option<u64>,
    // TODO: get rid of this --->
    simulation_window: Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)>,
    event_run_log: Option<Rc<dyn RunLogReader>>,
    trigger: TriggerConfig,
    generator_builder: Option<Box<dyn GeneratorBuilder>>,
}

impl EffectBuilder {
    fn new(key: String) -> Self {
        EffectBuilder {
            key,
            start: None,
            end: None,
            limit: None,
            now: None,
            prototype_key: None,
            prototype: None,
            seed: None,
            simulation_window: None,
            event_run_log: None,
            trigger: TriggerConfig::ClockExpression("hz(1, day)".into()),
            generator_builder: None,
        }
    }

    pub fn start(mut self, start: impl Into<Bound>) -> Self {
        self.set_start(start);
        self
    }

    /// Sets `start` to a [`Bound`]; a string is a CEL expression, parsed at build so it can refer to the default prototype via `simulation`.
    pub fn set_start(&mut self, start: impl Into<Bound>) -> &mut Self {
        let start: Bound = start.into();
        self.start = Some(start.of_kind(BoundKind::Start));
        self
    }

    pub fn end(mut self, end: impl Into<Bound>) -> Self {
        self.set_end(end);
        self
    }

    /// Sets `end` to a [`Bound`]; a string is a CEL expression, parsed at build so it can refer to the default prototype via `simulation`.
    pub fn set_end(&mut self, end: impl Into<Bound>) -> &mut Self {
        let end: Bound = end.into();
        self.end = Some(end.of_kind(BoundKind::End));
        self
    }

    pub fn limit(mut self, limit: NonZeroU64) -> Self {
        self.set_limit(limit);
        self
    }

    pub fn set_limit(&mut self, limit: NonZeroU64) -> &mut Self {
        self.limit = Some(limit);
        self
    }

    pub fn prototype_key(mut self, key: impl Into<String>) -> Self {
        self.set_prototype_key(key);
        self
    }

    /// Sets the key of the prototype this effect names; no key means the default prototype and a key matching no prototype an empty one.
    pub fn set_prototype_key(&mut self, key: impl Into<String>) -> &mut Self {
        self.prototype_key = Some(key.into());
        self
    }

    pub fn prototype(mut self, prototype: &Prototype) -> Self {
        self.set_prototype(prototype);
        self
    }

    /// Sets the prototype whose `start`, `end` and `limit` the effect takes where it sets none itself; without one the prototype is empty.
    pub fn set_prototype(&mut self, prototype: &Prototype) -> &mut Self {
        self.prototype = Some(prototype.clone());
        self
    }

    pub fn now(mut self, now: DateTime<FixedOffset>) -> Self {
        self.set_now(now);
        self
    }

    /// Sets the instant relative moments resolve against; defaults to the current time at build.
    pub fn set_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.now = Some(now);
        self
    }

    /// Sets the simulation's start and end, which generators see; without them they default to the effect's own start and end.
    pub fn set_simulation_window(
        &mut self,
        start: DateTime<FixedOffset>,
        end: DateTime<FixedOffset>,
    ) -> &mut Self {
        self.simulation_window = Some((start, end));
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

    /// The key of the prototype this effect asked for, if any.
    pub fn get_prototype_key(&self) -> Option<&str> {
        self.prototype_key.as_deref()
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

    /// Lays the effect's own `start`, `end` and `limit` over its prototype's and builds it; errors unless the window has a `start` and an `end` or `limit`, and a `start` before any `end`.
    pub fn build(self) -> Result<Effect, Vec<BuildError>> {
        let now = self.now.unwrap_or_else(|| Utc::now().fixed_offset());
        let prototype = self.prototype.as_ref();
        let mut errors: Vec<BuildError> = vec![];
        let own_start =
            self.start
                .as_ref()
                .map(|bound| bound.resolve(now, prototype))
                .transpose()
                .map_err(|e| {
                    errors.extend(e.iter().map(|e| {
                        BuildError::effect(self.key.clone(), EffectKey::Start, e.message())
                    }))
                })
                .ok()
                .flatten();
        let own_end = self
            .end
            .as_ref()
            .map(|bound| bound.resolve(now, prototype))
            .transpose()
            .map_err(|e| {
                errors.extend(
                    e.iter()
                        .map(|e| BuildError::effect(self.key.clone(), EffectKey::End, e.message())),
                )
            })
            .ok()
            .flatten();
        if !errors.is_empty() {
            return Err(errors);
        }
        let start = own_start.or(prototype.and_then(|p| p.start));
        let end = own_end.or(prototype.and_then(|p| p.end));
        let limit = self
            .limit
            .or(prototype.and_then(|p| p.limit))
            .map(NonZeroU64::get);

        if start.is_none() {
            errors.push(BuildError::effect(
                self.key.clone(),
                EffectKey::Start,
                "start must be set on the effect or its prototype",
            ));
        }
        if end.is_none() && limit.is_none() {
            errors.push(BuildError::effect(
                self.key.clone(),
                EffectKey::End,
                "end or limit must be set on the effect or its prototype",
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
        let Some(start) = start.filter(|_| errors.is_empty()) else {
            return Err(errors);
        };

        let seed = self.seed.unwrap_or(1);
        let origin = start;
        let (simulation_start, simulation_end) = self
            .simulation_window
            .unwrap_or((start, end.unwrap_or(start)));

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

        let trigger_result = build_trigger(
            self.trigger.clone(),
            &self.key,
            seed,
            start,
            origin,
            &run_log_reader,
        );

        match generator_result
            .and_try(trigger_result)
            .map_err(|e| e.into_iter().flatten().collect::<Vec<_>>())
        {
            Ok((generator, trigger)) if errors.is_empty() => Ok(Effect {
                key: self.key,
                run_log_reader: run_log_reader.clone(),
                trigger,
                trigger_config: self.trigger,
                seed,
                generator,
                end_offset: end.map(|end| offset_from(end, origin)),
                limit,
                produced: 0,
                origin,
                start,
                end,
                prototype: self.prototype_key,
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

fn offset_from(instant: DateTime<FixedOffset>, origin: DateTime<FixedOffset>) -> i64 {
    (instant - origin).num_milliseconds().max(0)
}

fn build_trigger(
    config: TriggerConfig,
    key: &str,
    seed: u64,
    start: DateTime<FixedOffset>,
    origin: DateTime<FixedOffset>,
    run_log_reader: &Rc<dyn RunLogReader>,
) -> Result<Trigger, Vec<BuildError>> {
    let clock = |builder: clock::ClockBuilder| {
        builder
            .key(key.to_string())
            .seed(seed)
            .start_offset(offset_from(start, origin))
            .build()
            .map(|mut clock| {
                let next_offset = clock.next();
                Trigger::Clock { clock, next_offset }
            })
    };

    match config {
        TriggerConfig::Effect { key } => Ok(Trigger::Effect {
            run_log_reader: run_log_reader.clone(),
            key,
            sim_start: origin,
            last_offset: 0,
        }),
        TriggerConfig::ClockHertz(hertz) => clock(Clock::builder().hertz(hertz)),
        TriggerConfig::ClockExpression(expression) => {
            clock(Clock::builder().expression(expression))
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
    use crate::effect::prototype::Prototype;
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
        let prototype = Prototype {
            start: Some(start),
            end: Some(now),
            ..Prototype::default()
        };
        builder
            .set_prototype(&prototype)
            .set_simulation_window(start, now);
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
