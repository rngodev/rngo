use crate::build::{BuildError, EffectKey};
use crate::effect::clock::Clock;
use crate::effect::schema::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext};
use crate::effect::trigger::{Trigger, TriggerConfig};
use crate::effect::{Input, SkippedInput};
use crate::run_log::{RunLogReader, SimpleEventRunLog};
use crate::util::ext::FlattenErr;
use crate::util::time::Moment;
use chrono::{DateTime, FixedOffset, TimeDelta};
use multi_try::MultiTry;
use std::rc::Rc;

#[derive(Debug)]
pub struct SourceEffect {
    pub key: String,
    run_log_reader: Rc<dyn RunLogReader>,
    trigger: Trigger,
    schema: Box<dyn Schema>,
    end_offset: u64,
    sim_start: DateTime<FixedOffset>,
    sim_end: DateTime<FixedOffset>,
}

impl SourceEffect {
    pub fn builder(key: String) -> EffectBuilder {
        EffectBuilder::new(key)
    }

    pub fn next_offset(&self) -> Option<u64> {
        let offset = self.trigger.next_offset()?;
        if offset > self.end_offset {
            None
        } else {
            Some(offset)
        }
    }
}

impl Iterator for SourceEffect {
    type Item = Result<Input, SkippedInput>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_offset()?;

        let trigger_event = self.trigger.pull()?;
        let context = SchemaContext {
            trigger: &trigger_event,
            simulation_start: self.sim_start,
            simulation_end: self.sim_end,
        };
        let offset = trigger_event.sim_offset;
        let timestamp = self.sim_start + TimeDelta::seconds(trigger_event.sim_offset as i64);

        let result = self.schema.next(&context);

        if let Some(data) = result.value {
            let last_id = self.run_log_reader.last().map(|e| e.id).unwrap_or(0);

            Some(Ok(Input {
                id: last_id + 1,
                effect: self.key.clone(),
                offset,
                timestamp,
                data,
                metadata: result.metadata,
            }))
        } else {
            Some(Err(SkippedInput {
                effect: self.key.clone(),
                offset,
                timestamp,
                metadata: result.metadata,
            }))
        }
    }
}

#[derive(Debug)]
pub struct EffectBuilder {
    pub key: String,
    pub start: Option<Moment>,
    pub end: Option<Moment>,
    now: Option<DateTime<FixedOffset>>,
    sim_start: Option<DateTime<FixedOffset>>,
    sim_end: Option<DateTime<FixedOffset>>,
    event_run_log: Option<Rc<dyn RunLogReader>>,
    seed: Option<u64>,
    trigger: TriggerConfig,
    schema_builder: Option<Box<dyn SchemaBuilder>>,
}

impl EffectBuilder {
    fn new(key: String) -> Self {
        EffectBuilder {
            key,
            start: None,
            end: None,
            now: None,
            sim_start: None,
            sim_end: None,
            event_run_log: None,
            seed: None,
            trigger: TriggerConfig::ClockExpression("hz(1, day)".into()),
            schema_builder: None,
        }
    }

    pub fn start(mut self, start: Moment) -> Self {
        self.set_start(start);
        self
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = Some(start);
        self
    }

    pub fn end(mut self, end: Moment) -> Self {
        self.set_end(end);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = Some(end);
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

    pub fn sim_start(mut self, start: DateTime<FixedOffset>) -> Self {
        self.set_sim_start(start);
        self
    }

    pub fn set_sim_start(&mut self, start: DateTime<FixedOffset>) -> &mut Self {
        self.sim_start = Some(start);
        self
    }

    pub fn sim_end(mut self, end: DateTime<FixedOffset>) -> Self {
        self.set_sim_end(end);
        self
    }

    pub fn set_sim_end(&mut self, end: DateTime<FixedOffset>) -> &mut Self {
        self.sim_end = Some(end);
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

    pub fn seed(mut self, seed: u64) -> Self {
        self.set_seed(seed);
        self
    }

    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = Some(seed);
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

    pub fn schema(mut self, builder: impl SchemaBuilder + 'static) -> Self {
        self.set_schema(builder);
        self
    }

    pub fn set_schema(&mut self, builder: impl SchemaBuilder + 'static) -> &mut Self {
        self.schema_builder = Some(Box::new(builder));
        self
    }

    pub fn build(self) -> Result<SourceEffect, Vec<BuildError>> {
        let Some(now) = self.now else {
            return Err(vec![BuildError::Effect {
                effect: self.key,
                key: EffectKey::Config,
                message: "now must be set via set_now()".into(),
            }]);
        };
        let seed = self.seed.unwrap_or(1);
        let run_log_reader: Rc<dyn RunLogReader> = self
            .event_run_log
            .unwrap_or_else(|| SimpleEventRunLog::new());
        let sim_start = self.sim_start.unwrap_or_else(|| now + TimeDelta::days(-30));
        let sim_end = self.sim_end.unwrap_or(now);
        let effect_end = self.end.map(|m| m.resolve(now)).unwrap_or(sim_end);
        let effect_start = self.start.map(|m| m.resolve(now)).unwrap_or(sim_start);
        let end_offset = (effect_end - sim_start).num_seconds().max(0) as u64;
        let start_offset = (effect_start - sim_start).num_seconds().max(0) as u64;

        let mut errors: Vec<BuildError> = vec![];

        if effect_start < sim_start {
            errors.push(BuildError::Effect {
                effect: self.key.clone(),
                key: EffectKey::Start,
                message: "start cannot be before simulation start".into(),
            });
        }
        if effect_end > sim_end {
            errors.push(BuildError::Effect {
                effect: self.key.clone(),
                key: EffectKey::End,
                message: "end cannot be after simulation end".into(),
            });
        }

        let schema_result = if let Some(schema_builder) = self.schema_builder {
            let visitor = SchemaBuildVisitor {
                event_run_log: run_log_reader.clone(),
                simulation_seed: seed,
                effect_key: self.key.clone(),
                path: vec![],
            };

            schema_builder.build(visitor)
        } else {
            Err(vec![BuildError::Effect {
                effect: self.key.clone(),
                key: EffectKey::Schema,
                message: "schema was not set".into(),
            }])
        };

        let trigger_result = match self.trigger {
            TriggerConfig::Effect { key } => Ok(Trigger::Effect {
                run_log_reader: run_log_reader.clone(),
                key,
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

        match schema_result.and_try(trigger_result).flatten_err() {
            Ok((schema, trigger)) if errors.is_empty() => Ok(SourceEffect {
                key: self.key,
                run_log_reader: run_log_reader.clone(),
                trigger,
                schema,
                end_offset,
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
