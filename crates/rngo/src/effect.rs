mod clock;
pub mod generator;
pub mod phase;
pub mod scenario;
pub mod simulation;
mod trigger;

use crate::build::{BuildError, EffectKey};
use crate::effect::phase::PhaseBuilder;
use crate::effect::scenario::ScenarioBuilder;
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
    end_offset: i64,
    limit: Option<u64>,
    produced: u64,
    origin: DateTime<FixedOffset>,
    sim_start: DateTime<FixedOffset>,
    sim_end: DateTime<FixedOffset>,
}

impl Effect {
    pub fn builder(key: String) -> EffectBuilder {
        EffectBuilder::new(key)
    }

    /// Milliseconds since simulation start at which this effect next fires.
    pub fn next_offset(&self) -> Option<i64> {
        if self.limit.is_some_and(|limit| self.produced >= limit) {
            return None;
        }

        let offset = self.trigger.next_offset()?;
        if offset > self.end_offset {
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
            simulation_start: self.sim_start,
            simulation_end: self.sim_end,
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
    window: PhaseBuilder,
    limit: Option<NonZeroU64>,
    now: Option<DateTime<FixedOffset>>,
    scenario: Option<Rc<ScenarioBuilder>>,
    phase: Option<String>,
    event_run_log: Option<Rc<dyn RunLogReader>>,
    trigger: TriggerConfig,
    generator_builder: Option<Box<dyn GeneratorBuilder>>,
}

impl EffectBuilder {
    fn new(key: String) -> Self {
        EffectBuilder {
            key,
            window: PhaseBuilder::default(),
            limit: None,
            now: None,
            scenario: None,
            phase: None,
            event_run_log: None,
            trigger: TriggerConfig::ClockExpression("hz(1, day)".into()),
            generator_builder: None,
        }
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.window.set_start(start);
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.window.set_end(end);
        self
    }

    /// Sets `start` from an expression, parsed when the effect builds so it can refer to the effect's phase via `simulation`; overrides `start`.
    pub fn set_start_expression(&mut self, expression: String) -> &mut Self {
        self.window.set_start_expression(expression);
        self
    }

    /// Sets `end` from an expression, parsed when the effect builds so it can refer to the effect's phase via `simulation`; overrides `end`.
    pub fn set_end_expression(&mut self, expression: String) -> &mut Self {
        self.window.set_end_expression(expression);
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

    pub fn now(mut self, now: DateTime<FixedOffset>) -> Self {
        self.set_now(now);
        self
    }

    pub fn set_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.now = Some(now);
        self
    }

    pub fn scenario(mut self, scenario: Rc<ScenarioBuilder>) -> Self {
        self.set_scenario(scenario);
        self
    }

    /// Sets the scenario supplying the seed and phases, built with `now` when the effect builds; defaults to an empty one.
    pub fn set_scenario(&mut self, scenario: Rc<ScenarioBuilder>) -> &mut Self {
        self.scenario = Some(scenario);
        self
    }

    pub(crate) fn set_default_scenario(&mut self, scenario: Rc<ScenarioBuilder>) {
        self.scenario.get_or_insert(scenario);
    }

    pub fn phase(mut self, name: impl Into<String>) -> Self {
        self.set_phase(name);
        self
    }

    /// Names the scenario phase whose window bounds this effect and its generators; defaults to the scenario's default phase.
    pub fn set_phase(&mut self, name: impl Into<String>) -> &mut Self {
        self.phase = Some(name.into());
        self
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

    pub fn build(self) -> Result<Effect, Vec<BuildError>> {
        let Some(now) = self.now else {
            return Err(vec![BuildError::effect(
                self.key,
                EffectKey::Config,
                "now must be set via set_now()",
            )]);
        };
        let scenario = self
            .scenario
            .as_deref()
            .cloned()
            .unwrap_or_default()
            .build(now)?;
        let seed = scenario.seed;
        let run_log_reader: Rc<dyn RunLogReader> = self
            .event_run_log
            .unwrap_or_else(|| SimpleEventRunLog::new());
        let phase = match &self.phase {
            Some(name) => match scenario.phase(Some(name)) {
                Some(phase) => phase.clone(),
                None => {
                    return Err(vec![BuildError::effect(
                        self.key,
                        EffectKey::Config,
                        format!("unknown phase \"{name}\""),
                    )]);
                }
            },
            None => scenario.default_phase.clone(),
        };
        let (sim_start, sim_end) = (phase.start, phase.end);
        let mut errors: Vec<BuildError> = vec![];
        let bounds = self.window.parent(&phase).for_effect(self.key.clone());
        let (effect_start, effect_end) = match bounds.build(now) {
            Ok(bounds) => (bounds.start, bounds.end),
            Err(mut e) => {
                errors.append(&mut e);
                (sim_start, sim_end)
            }
        };
        let origin = scenario.origin();
        let end_offset = (effect_end - origin).num_milliseconds().max(0);
        let start_offset = (effect_start - origin).num_milliseconds().max(0);

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
                .start_offset(start_offset)
                .build()
                .map(|mut clock| {
                    let next_offset = clock.next();
                    Trigger::Clock { clock, next_offset }
                }),
            TriggerConfig::ClockExpression(expression) => Clock::builder()
                .key(self.key.clone())
                .seed(seed)
                .expression(expression)
                .start_offset(start_offset)
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
                end_offset,
                limit: self.limit.map(NonZeroU64::get),
                produced: 0,
                origin,
                sim_start,
                sim_end,
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
    use super::Effect;
    use crate::build::{BuildError, constant};
    use crate::effect::generator::{
        Generator, GeneratorBuildVisitor, GeneratorBuilder, GeneratorContext, GeneratorResult,
        Metadata,
    };
    use chrono::Utc;
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

    #[test]
    fn limit_caps_produced_inputs() {
        let effect = Effect::builder("ping".into())
            .now(Utc::now().fixed_offset())
            .trigger_hertz(1000.0)
            .limit(NonZeroU64::new(3).unwrap())
            .generator(constant().value(1))
            .build()
            .unwrap();

        let results: Vec<_> = effect.collect();

        assert_eq!(results.len(), 3);
        assert!(results.iter().all(Result::is_ok));
    }

    #[test]
    fn limit_does_not_count_skipped_inputs() {
        let effect = Effect::builder("alternating".into())
            .now(Utc::now().fixed_offset())
            .trigger_hertz(1000.0)
            .limit(NonZeroU64::new(3).unwrap())
            .generator(AlternatingGeneratorBuilder)
            .build()
            .unwrap();

        let results: Vec<_> = effect.collect();

        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 3);
        assert_eq!(results.iter().filter(|r| r.is_err()).count(), 3);
    }
}
