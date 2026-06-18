use super::clock::Clock;
use crate::event::{Event, EventLogIndex};
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum TriggerConfig {
    Effect { key: String },
    ClockHertz(f64),
    ClockExpression(String),
}

pub struct TriggerEvent {
    pub offset: u64,
    pub event: Option<Rc<Event>>,
}

#[derive(Debug)]
pub enum Trigger {
    Effect {
        index: Box<dyn EventLogIndex>,
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
                if let Some(event) = index.sample() {
                    match *event {
                        Event::Effect { offset, .. } => {
                            if &offset > last_offset {
                                Some(offset)
                            } else {
                                None
                            }
                        }
                        Event::Error { .. } => None,
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
                if let Some(event) = index.sample() {
                    match *event {
                        Event::Effect { offset, .. } => {
                            *last_offset = offset;
                            Some(TriggerEvent {
                                offset,
                                event: Some(event.clone()),
                            })
                        }
                        Event::Error { .. } => None,
                    }
                } else {
                    None
                }
            }
            Trigger::Clock { clock, next_offset } => {
                if let Some(offset) = next_offset {
                    let event = TriggerEvent {
                        offset: *offset,
                        event: None,
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
