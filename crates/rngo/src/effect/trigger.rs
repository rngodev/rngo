use super::clock::Clock;
use crate::RunError;
use crate::effect::Input;
use crate::log::RunLogReader;
use chrono::{DateTime, FixedOffset};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub(super) enum TriggerConfig {
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
pub(super) enum Trigger {
    Effect {
        run_log_reader: Rc<dyn RunLogReader>,
        key: String,
        sim_start: DateTime<FixedOffset>,
        last_offset: i64,
    },
    Clock {
        clock: Clock,
        /// The clock's next item: an offset, or the error computing it, which surfaces at the last offset.
        next: Option<Result<i64, RunError>>,
    },
}

impl Trigger {
    pub(super) fn next_offset(&self) -> Option<i64> {
        match &self {
            Trigger::Clock { clock, next } => match next.as_ref()? {
                Ok(offset) => Some(*offset),
                Err(_) => Some(clock.last()),
            },
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

    /// Takes the next trigger event, or the error that stops this trigger for good.
    pub(super) fn pull(&mut self) -> Option<Result<TriggerEvent, RunError>> {
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
                    Some(Ok(TriggerEvent {
                        sim_offset: offset,
                        input_event: Some(input_event.clone()),
                    }))
                } else {
                    None
                }
            }
            Trigger::Clock { clock, next } => match next.take()? {
                Ok(offset) => {
                    *next = clock.next();
                    Some(Ok(TriggerEvent {
                        sim_offset: offset,
                        input_event: None,
                    }))
                }
                Err(error) => Some(Err(error)),
            },
        }
    }
}
