use crate::log::{LogIndex, LogIndexConfig, LogReader};
use crate::{Input, Log, LogEvent};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct SimpleEventLogReader {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
}

impl LogReader for SimpleEventLogReader {
    fn last(&self) -> Option<Rc<Input>> {
        self.input_events.borrow().last().cloned()
    }

    fn index(&self, config: LogIndexConfig) -> Box<dyn LogIndex> {
        Box::new(SimpleEventLogIndex {
            input_events: Rc::clone(&self.input_events),
            config,
        })
    }
}

#[derive(Default, Debug)]
pub struct SimpleEventLog {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
}

impl Log for SimpleEventLog {
    fn push(&mut self, event: LogEvent) {
        if let LogEvent::Input(input_event) = event {
            self.input_events.borrow_mut().push(Rc::new(input_event));
        }
    }

    fn reader(&self) -> Rc<dyn LogReader> {
        Rc::new(SimpleEventLogReader {
            input_events: Rc::clone(&self.input_events),
        })
    }
}

#[derive(Debug)]
pub struct SimpleEventLogIndex {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
    config: LogIndexConfig,
}

impl LogIndex for SimpleEventLogIndex {
    fn sample(&self) -> Option<Rc<Input>> {
        let input_events = self.input_events.borrow();

        let mut filtered_events = input_events.iter().filter(|e| match &self.config {
            LogIndexConfig::ByEffect {
                key: config_key, ..
            } => &e.effect == config_key,
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
