mod clock;
mod trigger;

use crate::build::{BuildError, EffectKey};
use crate::format::Format;
use crate::log::{Log, LogIndexConfig, LogReader, SimpleEventLog};
use crate::schema::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::util::ext::FlattenErr;
use crate::util::time::Moment;
use chrono::{DateTime, FixedOffset, TimeDelta};
use clock::Clock;
use multi_try::MultiTry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::rc::Rc;
use trigger::{Trigger, TriggerConfig};

pub use trigger::TriggerEvent;

#[derive(Debug)]
pub struct Effect {
    key: String,
    event_log: Rc<dyn LogReader>,
    trigger: Trigger,
    schema: Box<dyn Schema>,
    end_offset: u64,
    format: Option<Box<dyn Format>>,
}

impl Effect {
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

impl Iterator for Effect {
    type Item = Result<EffectEvent, String>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_offset()?;

        let trigger_event = self.trigger.pull()?;
        let context = SchemaContext {
            trigger: &trigger_event,
        };

        let last_id = self.event_log.last().map(|e| e.id).unwrap_or(0);
        match self.schema.next(&context) {
            SchemaResult::Ok { value } => Some(Ok(EffectEvent {
                id: last_id + 1,
                key: self.key.clone(),
                offset: trigger_event.offset,
                format: self.format.as_ref().map(|f| f.format(&value)),
                value,
            })),
            SchemaResult::Err(message) => Some(Err(message)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectEvent {
    pub id: u64,
    pub key: String,
    pub offset: u64,
    pub value: Value,
    pub format: Option<String>,
}

#[derive(Debug)]
pub struct EffectBuilder {
    pub key: String,
    pub start: Option<Moment>,
    pub end: Option<Moment>,
    now: Option<DateTime<FixedOffset>>,
    sim_start: Option<DateTime<FixedOffset>>,
    sim_end: Option<DateTime<FixedOffset>>,
    event_log: Option<Rc<dyn LogReader>>,
    seed: Option<u64>,
    trigger: TriggerConfig,
    schema_builder: Option<Box<dyn SchemaBuilder>>,
    format: Option<Box<dyn Format>>,
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
            event_log: None,
            seed: None,
            trigger: TriggerConfig::ClockExpression("1.0 / day".into()),
            schema_builder: None,
            format: None,
        }
    }

    pub fn set_start(&mut self, start: Moment) -> &mut Self {
        self.start = Some(start);
        self
    }

    pub fn set_end(&mut self, end: Moment) -> &mut Self {
        self.end = Some(end);
        self
    }

    pub fn set_now(&mut self, now: DateTime<FixedOffset>) -> &mut Self {
        self.now = Some(now);
        self
    }

    pub fn set_sim_start(&mut self, start: DateTime<FixedOffset>) -> &mut Self {
        self.sim_start = Some(start);
        self
    }

    pub fn set_sim_end(&mut self, end: DateTime<FixedOffset>) -> &mut Self {
        self.sim_end = Some(end);
        self
    }

    pub fn set_event_log(&mut self, event_log: Rc<dyn LogReader>) -> &mut Self {
        self.event_log = Some(event_log);
        self
    }

    pub fn set_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = Some(seed);
        self
    }

    pub fn set_trigger_effect(&mut self, key: String) -> &mut Self {
        self.trigger = TriggerConfig::Effect { key };
        self
    }

    pub fn set_trigger_hertz(&mut self, hertz: f64) -> &mut Self {
        self.trigger = TriggerConfig::ClockHertz(hertz);
        self
    }

    pub fn set_trigger_expression(&mut self, expression: String) -> &mut Self {
        self.trigger = TriggerConfig::ClockExpression(expression);
        self
    }

    pub fn set_schema(&mut self, builder: impl SchemaBuilder + 'static) -> &mut Self {
        self.schema_builder = Some(Box::new(builder));
        self
    }

    pub fn set_format(&mut self, format: Box<dyn Format>) -> &mut Self {
        self.format = Some(format);
        self
    }

    pub fn build(self) -> Result<Effect, Vec<BuildError>> {
        let Some(now) = self.now else {
            return Err(vec![BuildError::Effect {
                effect: self.key,
                key: EffectKey::Config,
                message: "now must be set via set_now()".into(),
            }]);
        };
        let event_log: Rc<dyn LogReader> = self
            .event_log
            .unwrap_or_else(|| SimpleEventLog::default().reader());
        let seed = self.seed.unwrap_or(1);
        let sim_start = self.sim_start.unwrap_or_else(|| now + TimeDelta::days(-30));
        let sim_end = self.sim_end.unwrap_or(now);
        let effect_end = self.end.map(|m| m.resolve(now)).unwrap_or(sim_end);
        let effect_start = self.start.map(|m| m.resolve(now)).unwrap_or(sim_start);
        let end_offset = (effect_end - sim_start).num_seconds().max(0) as u64;
        let start_offset = (effect_start - sim_start).num_seconds().max(0) as u64;

        let schema_result = if let Some(schema_builder) = self.schema_builder {
            let visitor = SchemaBuildVisitor {
                event_log: event_log.clone(),
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
            TriggerConfig::Effect { key } => {
                let index = event_log.index(LogIndexConfig::ByEffect {
                    key: key.clone(),
                    last_only: true,
                });
                Ok(Trigger::Effect {
                    index,
                    last_offset: 0,
                })
            }
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

        let (schema, trigger) = schema_result.and_try(trigger_result).flatten_err()?;

        Ok(Effect {
            key: self.key,
            event_log: event_log.clone(),
            trigger,
            schema,
            end_offset,
            format: self.format,
        })
    }
}
