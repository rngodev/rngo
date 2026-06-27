use crate::Signal;
use crate::effect::EffectEvent;
use std::cell::RefCell;
use std::rc::Rc;

pub trait EventLog: std::fmt::Debug {
    fn push_effect(&self, event: &EffectEvent);
    fn push_error(&self, event: &str);
    fn push_signal(&self, event: &Signal);
    fn last(&self) -> Option<Rc<EffectEvent>>;
    fn index(&self, config: EventLogIndexConfig) -> Box<dyn EventLogIndex>;
}

pub trait EventLogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<EffectEvent>>;
}

#[derive(Clone, Debug)]
pub enum EventLogIndexConfig {
    ByEffect { key: String, last_only: bool },
}

#[derive(Default, Debug)]
pub struct SimpleEventLog {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
}

#[derive(Debug)]
pub struct SimpleEventLogIndex {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
    config: EventLogIndexConfig,
}

impl EventLogIndex for SimpleEventLogIndex {
    fn sample(&self) -> Option<Rc<EffectEvent>> {
        let effect_events = self.effect_events.borrow();

        let mut filtered_events = effect_events.iter().filter(|e| match &self.config {
            EventLogIndexConfig::ByEffect {
                key: config_key, ..
            } => &e.key == config_key,
        });

        match &self.config {
            EventLogIndexConfig::ByEffect { last_only, .. } => {
                if *last_only {
                    filtered_events.next_back().cloned()
                } else {
                    let filtered_events = filtered_events.collect::<Vec<_>>();
                    if filtered_events.is_empty() {
                        None
                    } else {
                        let idx = fastrand::usize(..filtered_events.len());
                        filtered_events.get(idx).cloned().cloned()
                    }
                }
            }
        }
    }
}

impl EventLog for SimpleEventLog {
    fn push_effect(&self, effect_event: &EffectEvent) {
        self.effect_events
            .borrow_mut()
            .push(Rc::new(effect_event.clone()));
    }

    fn push_error(&self, _event: &str) {}

    fn push_signal(&self, _event: &Signal) {}

    fn last(&self) -> Option<Rc<EffectEvent>> {
        self.effect_events.borrow().last().cloned()
    }

    fn index(&self, config: EventLogIndexConfig) -> Box<dyn EventLogIndex> {
        Box::new(SimpleEventLogIndex {
            effect_events: Rc::clone(&self.effect_events),
            config,
        })
    }
}
