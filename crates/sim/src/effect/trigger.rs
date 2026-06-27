use super::clock::Clock;
use crate::effect::EffectEvent;
use crate::log::EventLogIndex;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub enum TriggerConfig {
    Effect { key: String },
    ClockHertz(f64),
    ClockExpression(String),
}

pub struct TriggerEvent {
    pub offset: u64,
    pub effect_event: Option<Rc<EffectEvent>>,
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
                if let Some(effect_event) = index.sample() {
                    if &effect_event.offset > last_offset {
                        Some(effect_event.offset)
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
                if let Some(effect_event) = index.sample() {
                    *last_offset = effect_event.offset;
                    Some(TriggerEvent {
                        offset: effect_event.offset,
                        effect_event: Some(effect_event.clone()),
                    })
                } else {
                    None
                }
            }
            Trigger::Clock { clock, next_offset } => {
                if let Some(offset) = next_offset {
                    let event = TriggerEvent {
                        offset: *offset,
                        effect_event: None,
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
