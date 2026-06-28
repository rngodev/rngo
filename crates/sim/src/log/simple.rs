use crate::log::{LogIndex, LogIndexConfig, LogReader};
use crate::{EffectEvent, Log, LogEvent};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct SimpleEventLogReader {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
}

impl LogReader for SimpleEventLogReader {
    fn last(&self) -> Option<Rc<EffectEvent>> {
        self.effect_events.borrow().last().cloned()
    }

    fn index(&self, config: LogIndexConfig) -> Box<dyn LogIndex> {
        Box::new(SimpleEventLogIndex {
            effect_events: Rc::clone(&self.effect_events),
            config,
        })
    }
}

#[derive(Default, Debug)]
pub struct SimpleEventLog {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
}

impl Log for SimpleEventLog {
    fn push(&mut self, event: LogEvent) {
        if let LogEvent::Effect(effect_event) = event {
            self.effect_events.borrow_mut().push(Rc::new(effect_event));
        }
    }

    fn reader(&self) -> Rc<dyn LogReader> {
        Rc::new(SimpleEventLogReader {
            effect_events: Rc::clone(&self.effect_events),
        })
    }
}

#[derive(Debug)]
pub struct SimpleEventLogIndex {
    effect_events: Rc<RefCell<Vec<Rc<EffectEvent>>>>,
    config: LogIndexConfig,
}

impl LogIndex for SimpleEventLogIndex {
    fn sample(&self) -> Option<Rc<EffectEvent>> {
        let effect_events = self.effect_events.borrow();

        let mut filtered_events = effect_events.iter().filter(|e| match &self.config {
            LogIndexConfig::ByEffect {
                key: config_key, ..
            } => &e.key == config_key,
        });

        match &self.config {
            LogIndexConfig::ByEffect { last_only, .. } => {
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
