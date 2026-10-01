use super::clock::Clock;
use crate::effect::Input;
use crate::log::RunLogReader;
use chrono::{DateTime, FixedOffset};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum TriggerConfig {
    Effect { key: String },
    ClockHertz(f64),
    ClockExpression(String),
}

pub struct TriggerEvent {
    /// Milliseconds since simulation start.
    pub sim_offset: i64,
    pub input_event: Option<Rc<Input>>,
}

#[derive(Debug)]
pub enum Trigger {
    Effect {
        run_log_reader: Rc<dyn RunLogReader>,
        key: String,
        sim_start: DateTime<FixedOffset>,
        last_offset: i64,
    },
    Clock {
        clock: Clock,
        next_offset: Option<i64>,
    },
}

impl Trigger {
    pub fn next_offset(&self) -> Option<i64> {
        match &self {
            Trigger::Clock { next_offset, .. } => *next_offset,
            Trigger::Effect {
                run_log_reader: event_run_log,
                key,
                sim_start,
                last_offset,
            } => {
                let input_event = event_run_log.last_for_effect(key)?;
                let offset = (input_event.timestamp - *sim_start).num_milliseconds();
                (offset > *last_offset).then_some(offset)
            }
        }
    }

    pub fn pull(&mut self) -> Option<TriggerEvent> {
        match self {
            Trigger::Effect {
                run_log_reader: event_run_log,
                key,
                sim_start,
                last_offset,
            } => {
                if let Some(input_event) = event_run_log.last_for_effect(key) {
                    let offset = (input_event.timestamp - *sim_start).num_milliseconds();
                    *last_offset = offset;
                    Some(TriggerEvent {
                        sim_offset: offset,
                        input_event: Some(input_event.clone()),
                    })
                } else {
                    None
                }
            }
            Trigger::Clock { clock, next_offset } => {
                if let Some(offset) = next_offset {
                    let event = TriggerEvent {
                        sim_offset: *offset,
                        input_event: None,
                    };

                    *next_offset = clock.next();

                    Some(event)
                } else {
                    None
                }
            }
        }
    }
}
