use crate::run_log::{RunLogIndex, RunLogIndexConfig, RunLogReader};
use crate::{Input, RunLog, RunLogEvent};
use rand::RngExt;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct SimpleEventRunLogReader {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
    rng: Rc<RefCell<Pcg32>>,
}

impl RunLogReader for SimpleEventRunLogReader {
    fn last(&self) -> Option<Rc<Input>> {
        self.input_events.borrow().last().cloned()
    }

    fn index(&self, config: RunLogIndexConfig) -> Box<dyn RunLogIndex> {
        Box::new(SimpleEventRunLogIndex {
            input_events: Rc::clone(&self.input_events),
            rng: Rc::clone(&self.rng),
            config,
        })
    }
}

/// An in-memory [`RunLog`], used as the default when a [`crate::Simulation`] isn't given an
/// on-disk one. Owns a `Pcg32` seeded from the simulation's seed, shared with every reader/index
/// it hands out, so [`RunLogIndex::sample`]'s random branch is reproducible for a given seed.
#[derive(Debug)]
pub struct SimpleEventRunLog {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
    rng: Rc<RefCell<Pcg32>>,
}

impl SimpleEventRunLog {
    pub fn new(seed: u64) -> Self {
        SimpleEventRunLog {
            input_events: Rc::new(RefCell::new(Vec::new())),
            rng: Rc::new(RefCell::new(
                Seeder::from(&format!("{seed}-run_log")).into_rng(),
            )),
        }
    }
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
            rng: Rc::clone(&self.rng),
        })
    }
}

#[derive(Debug)]
pub struct SimpleEventRunLogIndex {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
    rng: Rc<RefCell<Pcg32>>,
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
                        let idx = self.rng.borrow_mut().random_range(0..filtered_events.len());
                        filtered_events.get(idx).cloned().cloned()
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Builds a `SimpleEventRunLog` under the given seed, populates it with ten inputs on effect
    /// "a", then samples that effect's index `draws` times, returning the sampled ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let mut run_log = SimpleEventRunLog::new(seed);
        let reader = run_log.reader();

        for i in 1..=10u64 {
            run_log.push(RunLogEvent::Input(Input {
                id: i,
                effect: "a".to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            }));
        }

        let index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            last_only: false,
        });

        (0..draws).map(|_| index.sample().unwrap().id).collect()
    }

    #[test]
    fn index_sample_is_deterministic_for_a_fixed_seed() {
        assert_eq!(sampled_ids(42, 5), sampled_ids(42, 5));
    }

    #[test]
    fn index_sample_differs_across_seeds() {
        assert_ne!(sampled_ids(1, 5), sampled_ids(2, 5));
    }
}
