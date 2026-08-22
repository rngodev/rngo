use super::clock::Clock;
use crate::effect::Input;
use crate::log::LogIndex;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum TriggerConfig {
    Effect { key: String },
    ClockHertz(f64),
    ClockExpression(String),
}

pub struct TriggerEvent {
    pub sim_offset: u64,
    pub input_event: Option<Rc<Input>>,
}

#[derive(Debug)]
pub enum Trigger {
    Effect {
        index: Box<dyn LogIndex>,
        last_offset: u64,
    },
    Clock {
        clock: Clock,
        next_offset: Option<u64>,
    },
}

impl Trigger {
    pub fn next_offset(&self) -> Option<u64> {
        match &self {
            Trigger::Clock { next_offset, .. } => *next_offset,
            Trigger::Effect {
                index, last_offset, ..
            } => {
                if let Some(input_event) = index.sample() {
                    if &input_event.offset > last_offset {
                        Some(input_event.offset)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
    }

    pub fn pull(&mut self) -> Option<TriggerEvent> {
        match self {
            Trigger::Effect {
                index, last_offset, ..
            } => {
                if let Some(input_event) = index.sample() {
                    *last_offset = input_event.offset;
                    Some(TriggerEvent {
                        sim_offset: input_event.offset,
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
