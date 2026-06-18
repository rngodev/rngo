mod clock;
mod trigger;

use crate::build::{BuildError, EffectKey};
use crate::event::{Event, EventLog, EventLogIndexConfig};
use crate::schema::{Schema, SchemaBuildVisitor, SchemaBuilder, SchemaContext, SchemaResult};
use crate::util::ext::FlattenErr;
use crate::util::time::Moment;
use clock::Clock;
use multi_try::MultiTry;
use std::rc::Rc;
use trigger::{Trigger, TriggerConfig};

pub use trigger::TriggerEvent;

#[derive(Debug)]
pub struct Effect {
    key: String,
    event_log: Rc<dyn EventLog>,
    trigger: Trigger,
    pub schema: Box<dyn Schema>,
}

impl Effect {
    pub fn builder(key: String) -> EffectBuilder {
        EffectBuilder::new(key)
    }

    pub fn next_offset(&self) -> Option<u64> {
        self.trigger.next_offset()
    }
}

impl Iterator for Effect {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        let trigger_event = self.trigger.pull()?;
        let context = SchemaContext {
            trigger: &trigger_event,
        };

        let id = self.event_log.last().map(|e| e.id() + 1).unwrap_or(1);
        match self.schema.next(&context) {
            SchemaResult::Ok { value } => Some(Event::Effect {
                key: self.key.clone(),
                id,
                offset: trigger_event.offset,
                value,
            }),
            SchemaResult::Err(message) => Some(Event::Error { id, message }),
        }
    }
}

#[derive(Debug)]
pub struct EffectBuilder {
    pub key: String,
    pub start: Option<Moment>,
    pub end: Option<Moment>,
    trigger: TriggerConfig,
    schema_builder: Option<Box<dyn SchemaBuilder>>,
}

impl EffectBuilder {
    fn new(key: String) -> Self {
        EffectBuilder {
            key,
            start: None,
            end: None,
            trigger: TriggerConfig::ClockExpression("1.0 / day".into()),
            schema_builder: None,
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

    pub fn build(
        self,
        event_log: &Rc<dyn EventLog>,
        simulation_seed: u64,
    ) -> Result<Effect, Vec<BuildError>> {
        let schema_result = if let Some(schema_builder) = self.schema_builder {
            let visitor = SchemaBuildVisitor {
                event_log: event_log.clone(),
                simulation_seed,
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
                let index = event_log.index(EventLogIndexConfig::ByEffect {
                    key: key.clone(),
                    last_only: true,
                });
                Ok(Trigger::Effect {
                    index,
                    last_offset: 0,
                })
            }
            TriggerConfig::ClockHertz(hertz) => {
                let mut clock = Clock::for_hertz(self.key.clone(), simulation_seed, hertz);
                let next_offset = clock.next();
                Ok(Trigger::Clock { clock, next_offset })
            }
            TriggerConfig::ClockExpression(expression) => {
                let clock = Clock::for_expression(self.key.clone(), simulation_seed, expression);

                clock.map(|mut clock| {
                    let next_offset = clock.next();
                    Trigger::Clock { clock, next_offset }
                })
            }
        };

        let (schema, trigger) = schema_result.and_try(trigger_result).flatten_err()?;

        Ok(Effect {
            key: self.key,
            event_log: event_log.clone(),
            trigger,
            schema,
        })
    }
}
