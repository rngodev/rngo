use super::Event;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub trait EventLog: std::fmt::Debug {
    fn push(&self, event: &Event);
    fn last(&self) -> Option<Rc<Event>>;
    fn index(&self, config: EventLogIndexConfig) -> Box<dyn EventLogIndex>;
}

pub trait EventLogIndex: std::fmt::Debug {
    fn sample(&self) -> Option<Rc<Event>>;
}

#[derive(Clone, Debug)]
pub enum EventLogIndexConfig {
    ByEffect { key: String, last_only: bool },
}

#[derive(Default, Debug)]
pub struct SimpleEventLog {
    events: Rc<RefCell<Vec<Rc<Event>>>>,
    last_id: Cell<u64>,
}

#[derive(Debug)]
pub struct SimpleEventLogIndex {
    events: Rc<RefCell<Vec<Rc<Event>>>>,
    config: EventLogIndexConfig,
}

impl EventLogIndex for SimpleEventLogIndex {
    fn sample(&self) -> Option<Rc<Event>> {
        let events = self.events.borrow();

        let filtered_events = events.iter().filter(|e| match &***e {
            Event::Effect { key: event_key, .. } => match &self.config {
                EventLogIndexConfig::ByEffect {
                    key: config_key, ..
                } => event_key == config_key,
            },
            Event::Error { .. } => false,
        });

        match &self.config {
            EventLogIndexConfig::ByEffect { last_only, .. } => {
                if *last_only {
                    filtered_events.last().cloned()
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
    fn push(&self, event: &Event) {
        self.last_id.set(event.id());
        self.events.borrow_mut().push(Rc::new(event.clone()));
    }

    fn last(&self) -> Option<Rc<Event>> {
        self.events.borrow().last().cloned()
    }

    fn index(&self, config: EventLogIndexConfig) -> Box<dyn EventLogIndex> {
        Box::new(SimpleEventLogIndex {
            events: Rc::clone(&self.events),
            config,
        })
    }
}
