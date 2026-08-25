use crate::run_log::{RunLogIndex, RunLogIndexConfig, RunLogReader};
use crate::{Input, RunLog, RunLogEvent};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct SimpleEventRunLogReader {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
}

impl RunLogReader for SimpleEventRunLogReader {
    fn last(&self) -> Option<Rc<Input>> {
        self.input_events.borrow().last().cloned()
    }

    fn index(&self, config: RunLogIndexConfig) -> Box<dyn RunLogIndex> {
        Box::new(SimpleEventRunLogIndex {
            input_events: Rc::clone(&self.input_events),
            config,
        })
    }
}

#[derive(Default, Debug)]
pub struct SimpleEventRunLog {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
}

impl RunLog for SimpleEventRunLog {
    fn push(&mut self, event: RunLogEvent) {
        if let RunLogEvent::Input(input_event) = event {
            self.input_events.borrow_mut().push(Rc::new(input_event));
        }
    }

    fn reader(&self) -> Rc<dyn RunLogReader> {
        Rc::new(SimpleEventRunLogReader {
            input_events: Rc::clone(&self.input_events),
        })
    }
}

#[derive(Debug)]
pub struct SimpleEventRunLogIndex {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
    config: RunLogIndexConfig,
}

impl RunLogIndex for SimpleEventRunLogIndex {
    fn sample(&self) -> Option<Rc<Input>> {
        let input_events = self.input_events.borrow();

        let mut filtered_events = input_events.iter().filter(|e| match &self.config {
            RunLogIndexConfig::ByEffect {
                key: config_key, ..
            } => &e.effect == config_key,
        });

        match &self.config {
            RunLogIndexConfig::ByEffect { last_only, .. } => {
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
