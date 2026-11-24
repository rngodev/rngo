use super::clock::Clock;
use crate::SimulationError;
use crate::effect::Input;
use crate::log::LogReader;
use jiff::Timestamp;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) enum TriggerConfig {
    Effect { key: String },
    ClockHertz(f64),
    ClockExpression(String),
}

/// One firing of an effect's trigger.
pub struct TriggerEvent {
    pub(crate) sim_offset: i64,
    pub(crate) input_event: Option<Arc<Input>>,
}

impl TriggerEvent {
    /// Milliseconds since the origin.
    pub fn sim_offset(&self) -> i64 {
        self.sim_offset
    }

    /// The input that fired this trigger, for an effect triggered by another effect.
    pub fn input(&self) -> Option<&Input> {
        self.input_event.as_deref()
    }
}

#[derive(Debug)]
pub(super) enum Trigger {
    Effect {
        log_reader: Arc<dyn LogReader>,
        key: String,
        sim_start: Timestamp,
        last_offset: i64,
    },
    Clock {
        clock: Clock,
        /// The clock's next item: an offset, or the error computing it, which surfaces at the last offset.
        next: Option<Result<i64, SimulationError>>,
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
                log_reader,
                key,
                sim_start,
                last_offset,
            } => {
                let input_event = log_reader.last_for_effect(key)?;
                let offset = input_event.timestamp.duration_since(*sim_start).as_millis() as i64;
                (offset > *last_offset).then_some(offset)
            }
        }
    }

    /// Takes the next trigger event, or the error that stops this trigger for good.
    pub(super) fn pull(&mut self) -> Option<Result<TriggerEvent, SimulationError>> {
        match self {
            Trigger::Effect {
                log_reader,
                key,
                sim_start,
                last_offset,
            } => {
                if let Some(input_event) = log_reader.last_for_effect(key) {
                    let offset =
                        input_event.timestamp.duration_since(*sim_start).as_millis() as i64;
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
