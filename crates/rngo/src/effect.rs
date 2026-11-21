pub(crate) mod bound;
mod clock;
pub mod generator;
pub(crate) mod prototype;
pub(crate) mod simulation;
mod trigger;

use crate::SimulationError;
use crate::effect::bound::{Bound, BoundKind};
use crate::effect::prototype::Prototype;
use crate::error::{BuildError, BuildErrors, EffectKey};
use crate::log::{Metadata, RunLogReader};
use crate::moment::offset_timestamp;
use clock::Clock;
use generator::{
    Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext,
    Metadata as GeneratorMetadata,
};
use jiff::{SignedDuration, Timestamp};
use multi_try::MultiTry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use trigger::{Trigger, TriggerConfig};

pub use trigger::TriggerEvent;

#[derive(Debug)]
pub(crate) struct Effect {
    pub(crate) key: String,
    run_log_reader: Arc<dyn RunLogReader>,
    trigger: Trigger,
    trigger_config: TriggerConfig,
    seed: u64,
    generator: Box<dyn Generator>,
    end_offset: Option<i64>,
    limit: Option<u64>,
    produced: u64,
    origin: Timestamp,
    pub(crate) start: Timestamp,
    pub(crate) end: Option<Timestamp>,
    pub(crate) channel: Option<String>,
}

impl Effect {
    pub(crate) fn builder(key: impl Into<String>) -> EffectBuilder {
        EffectBuilder::new(key.into())
    }

    /// Moves the instant offsets are measured from, rebuilding the trigger around it; call before iterating, once every effect of the simulation is built and the earliest start is known.
    pub(crate) fn set_origin(&mut self, origin: Timestamp) -> Result<&mut Self, BuildErrors> {
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
    pub(crate) fn next_offset(&self) -> Option<i64> {
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
    type Item = Result<Step, SimulationError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_offset()?;

        let trigger_event = match self.trigger.pull()? {
            Ok(trigger_event) => trigger_event,
            Err(error) => return Some(Err(error)),
        };
        let context = GeneratorContext {
            trigger: &trigger_event,
            origin: self.origin,
        };
        let timestamp = offset_timestamp(
            self.origin,
            SignedDuration::from_millis(trigger_event.sim_offset),
        );

        let result = self.generator.next(&context);

        if let Some(data) = result.value {
            let last_id = self.run_log_reader.last().map(|e| e.id).unwrap_or(0);
            self.produced += 1;

            Some(Ok(Step::Input(Input {
                id: last_id + 1,
                effect: self.key.clone(),
                timestamp,
                data,
                metadata: result.metadata,
            })))
        } else {
            Some(Ok(Step::Skipped(SkippedInput {
                effect: self.key.clone(),
                timestamp,
                metadata: result.metadata,
            })))
        }
    }
}

/// What an effect did each time it fired.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Step {
    /// It produced an input.
    Input(Input),
    /// Its generator produced no value, so the attempt was skipped.
    Skipped(SkippedInput),
}

impl Step {
    /// The input, if the effect produced one.
    pub fn into_input(self) -> Option<Input> {
        match self {
            Step::Input(input) => Some(input),
            Step::Skipped(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Input {
    pub id: u64,
    pub effect: String,
    pub timestamp: Timestamp,
    pub data: Value,
    pub metadata: Vec<GeneratorMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedInput {
    pub effect: String,
    pub timestamp: Timestamp,
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

/// What the simulation hands an effect when it builds it.
pub(crate) struct EffectContext<'a> {
    pub(crate) now: Timestamp,
    pub(crate) seed: u64,
    pub(crate) prototype: &'a Prototype,
    pub(crate) run_log_reader: Arc<dyn RunLogReader>,
}

/// Describes an effect: its window, channel, prototype, trigger and generator; the simulation fills in the rest when it builds.
#[derive(Debug)]
pub struct EffectBuilder {
    key: String,
    start: Option<Bound>,
    end: Option<Bound>,
    limit: Option<u64>,
    channel: Option<String>,
    prototype_key: Option<String>,
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
            channel: None,
            prototype_key: None,
            trigger: TriggerConfig::ClockExpression("hz(1, day)".into()),
            generator_builder: None,
        }
    }

    /// When the effect starts firing, overriding its prototype's start.
    pub fn start(mut self, start: impl Into<Bound>) -> Self {
        let start: Bound = start.into();
        self.start = Some(start.of_kind(BoundKind::Start));
        self
    }

    /// When the effect stops firing, overriding its prototype's end.
    pub fn end(mut self, end: impl Into<Bound>) -> Self {
        let end: Bound = end.into();
        self.end = Some(end.of_kind(BoundKind::End));
        self
    }

    /// The most inputs the effect produces, overriding its prototype's limit; skipped attempts don't count. Must be at least 1.
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// The key of the channel the effect writes to, overriding its prototype's.
    pub fn channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    /// The name of the prototype the effect takes its defaults from; without one it uses the default prototype.
    pub fn prototype_key(mut self, key: impl Into<String>) -> Self {
        self.prototype_key = Some(key.into());
        self
    }

    /// The key of the prototype this effect asked for, if any.
    pub(crate) fn get_prototype_key(&self) -> Option<&str> {
        self.prototype_key.as_deref()
    }

    /// Fires the effect once for each input of the effect `key`.
    pub fn trigger_effect(mut self, key: impl Into<String>) -> Self {
        self.trigger = TriggerConfig::Effect { key: key.into() };
        self
    }

    /// Fires the effect at a fixed rate, in inputs per second.
    pub fn trigger_hertz(mut self, hertz: f64) -> Self {
        self.trigger = TriggerConfig::ClockHertz(hertz);
        self
    }

    /// Fires the effect at the rate a CEL expression returns, such as `hz(1, day)`; the default.
    pub fn trigger_expression(mut self, expression: impl Into<String>) -> Self {
        self.trigger = TriggerConfig::ClockExpression(expression.into());
        self
    }

    /// Generates the data of each input.
    pub fn generator(mut self, builder: impl GeneratorBuilder + 'static) -> Self {
        self.generator_builder = Some(Box::new(builder));
        self
    }

    /// Lays the effect's own `start`, `end` and `limit` over its prototype's and builds it; errors unless the window has a `start` and an `end` or `limit`, a `start` before any `end`, and a nonzero `limit`.
    pub(crate) fn build(self, context: EffectContext) -> Result<Effect, BuildErrors> {
        let EffectContext {
            now,
            seed,
            prototype,
            run_log_reader,
        } = context;
        let mut errors: Vec<BuildError> = vec![];
        if self.limit == Some(0) {
            errors.push(BuildError::effect(
                self.key.clone(),
                EffectKey::Limit,
                "limit must be at least 1",
            ));
        }
        let own_start =
            self.start
                .as_ref()
                .map(|bound| bound.resolve(now, Some(prototype)))
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
            .map(|bound| bound.resolve(now, Some(prototype)))
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
            return Err(errors.into());
        }
        let start = own_start.or(prototype.start);
        let end = own_end.or(prototype.end);
        let limit = self.limit.or(prototype.limit);

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
            return Err(errors.into());
        };

        let origin = start;

        let generator_result = if let Some(generator_builder) = self.generator_builder {
            let visitor = GeneratorBuildVisitor {
                event_run_log: run_log_reader.clone(),
                simulation_seed: seed,
                effect_key: self.key.clone(),
                path: vec![],
                spec_path: vec!["effects".into(), self.key.clone(), "generator".into()],
            };

            generator_builder.build(visitor)
        } else {
            Err(BuildError::effect(
                self.key.clone(),
                EffectKey::Generator,
                "generator was not set",
            )
            .into())
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
                channel: self.channel.or_else(|| prototype.channel.clone()),
            }),
            Ok(_) => Err(errors.into()),
            Err(e) => {
                errors.extend(e);
                Err(errors.into())
            }
        }
    }
}

fn offset_from(instant: Timestamp, origin: Timestamp) -> i64 {
    (instant.duration_since(origin).as_millis() as i64).max(0)
}

fn build_trigger(
    config: TriggerConfig,
    key: &str,
    seed: u64,
    start: Timestamp,
    origin: Timestamp,
    run_log_reader: &Arc<dyn RunLogReader>,
) -> Result<Trigger, BuildErrors> {
    let clock = |builder: clock::ClockBuilder| {
        builder
            .key(key.to_string())
            .seed(seed)
            .start_offset(offset_from(start, origin))
            .build()
            .map(|mut clock| {
                let next = clock.next();
                Trigger::Clock { clock, next }
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
    use super::{Effect, EffectBuilder, EffectContext};
    use crate::build::constant;
    use crate::effect::generator::{
        Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
        Metadata,
    };
    use crate::effect::prototype::Prototype;
    use crate::error::BuildErrors;
    use crate::log::SimpleEventRunLog;
    use jiff::{SignedDuration, Timestamp};
    use std::sync::Arc;

    fn context(prototype: &Prototype) -> EffectContext<'_> {
        EffectContext {
            now: Timestamp::now(),
            seed: 1,
            prototype,
            run_log_reader: Arc::new(SimpleEventRunLog::new()),
        }
    }

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
        ) -> Result<Box<dyn Generator>, BuildErrors> {
            Ok(Box::new(AlternatingGenerator::default()))
        }
    }

    fn build(builder: EffectBuilder) -> Effect {
        let now = Timestamp::now();
        let start = now - SignedDuration::from_hours(24 * 30);
        let prototype = Prototype {
            start: Some(start),
            end: Some(now),
            ..Prototype::default()
        };
        builder.build(context(&prototype)).unwrap()
    }

    #[test]
    fn channel_comes_from_the_effect_else_its_prototype() {
        let now = Timestamp::now();
        let prototype = Prototype {
            start: Some(now - SignedDuration::from_hours(24)),
            end: Some(now),
            channel: Some("db".into()),
            ..Prototype::default()
        };
        let built = |channel: Option<&str>| {
            let mut builder = Effect::builder("ping").generator(constant().value(1));
            if let Some(channel) = channel {
                builder = builder.channel(channel);
            }
            builder.build(context(&prototype)).unwrap()
        };

        assert_eq!(built(None).channel.as_deref(), Some("db"));
        assert_eq!(built(Some("api")).channel.as_deref(), Some("api"));
    }

    #[test]
    fn limit_caps_produced_inputs() {
        let effect = build(
            Effect::builder("ping")
                .trigger_hertz(1000.0)
                .limit(3)
                .generator(constant().value(1)),
        );

        let results: Vec<_> = effect.collect();

        assert_eq!(results.len(), 3);
        assert!(
            results
                .iter()
                .all(|r| matches!(r, Ok(super::Step::Input(_))))
        );
    }

    #[test]
    fn limit_does_not_count_skipped_inputs() {
        let effect = build(
            Effect::builder("alternating")
                .trigger_hertz(1000.0)
                .limit(3)
                .generator(AlternatingGeneratorBuilder),
        );

        let results: Vec<_> = effect.collect();

        let inputs = results
            .iter()
            .filter(|r| matches!(r, Ok(super::Step::Input(_))))
            .count();
        let skipped = results
            .iter()
            .filter(|r| matches!(r, Ok(super::Step::Skipped(_))))
            .count();
        assert_eq!(inputs, 3);
        assert_eq!(skipped, 3);
    }
}
