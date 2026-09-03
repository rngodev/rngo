use crate::run_log::{Cursor, RunLogIndex, RunLogIndexConfig, RunLogReader};
use crate::signal::{SignalError, SignalOutcome};
use crate::{Input, RunLog, RunLogEvent, spec};
use indexmap::IndexMap;
use rand::RngExt;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;
use std::cell::RefCell;
use std::collections::HashSet;
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
            returned: RefCell::new(HashSet::new()),
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

    /// Signals are SQL queries, and this in-memory log has no query engine to run them against,
    /// so every signal comes back as unsupported rather than silently evaluating nothing.
    fn evaluate_signals(
        &self,
        signals: &IndexMap<String, spec::Signal>,
    ) -> IndexMap<String, SignalOutcome> {
        signals
            .keys()
            .map(|key| {
                (
                    key.clone(),
                    SignalOutcome::error(SignalError::Unsupported { key: key.clone() }),
                )
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct SimpleEventRunLogIndex {
    input_events: Rc<RefCell<Vec<Rc<Input>>>>,
    rng: Rc<RefCell<Pcg32>>,
    /// Ids already handed out by this index under [`Cursor::Unique`]; empty and unused otherwise.
    returned: RefCell<HashSet<u64>>,
    config: RunLogIndexConfig,
}

impl RunLogIndex for SimpleEventRunLogIndex {
    fn sample(&self) -> Option<Rc<Input>> {
        let input_events = self.input_events.borrow();

        let RunLogIndexConfig::ByEffect { key, cursor } = &self.config;

        let mut filtered_events = input_events.iter().filter(|e| &e.effect == key);

        match cursor {
            Cursor::Last => filtered_events.next_back().cloned(),
            Cursor::Random => {
                let filtered_events = filtered_events.collect::<Vec<_>>();
                if filtered_events.is_empty() {
                    None
                } else {
                    let idx = self.rng.borrow_mut().random_range(0..filtered_events.len());
                    filtered_events.get(idx).cloned().cloned()
                }
            }
            Cursor::Unique => {
                let returned = self.returned.borrow();
                let candidates = filtered_events
                    .filter(|e| !returned.contains(&e.id))
                    .collect::<Vec<_>>();
                drop(returned);

                if candidates.is_empty() {
                    None
                } else {
                    let idx = self.rng.borrow_mut().random_range(0..candidates.len());
                    let chosen = candidates.get(idx).cloned().cloned();
                    if let Some(chosen) = &chosen {
                        self.returned.borrow_mut().insert(chosen.id);
                    }
                    chosen
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
            cursor: Cursor::Random,
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

    fn push_inputs(run_log: &mut SimpleEventRunLog, effect: &str, count: u64) {
        for i in 1..=count {
            run_log.push(RunLogEvent::Input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            }));
        }
    }

    #[test]
    fn unique_cursor_never_repeats_and_exhausts() {
        let mut run_log = SimpleEventRunLog::new(1);
        let reader = run_log.reader();
        push_inputs(&mut run_log, "a", 5);

        let index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });

        let mut seen = HashSet::new();
        for _ in 0..5 {
            let sampled = index.sample().unwrap();
            assert!(
                seen.insert(sampled.id),
                "id {} returned more than once",
                sampled.id
            );
        }

        assert!(index.sample().is_none());
    }

    #[test]
    fn unique_cursor_is_deterministic_for_a_fixed_seed() {
        fn draw_all(seed: u64) -> Vec<u64> {
            let mut run_log = SimpleEventRunLog::new(seed);
            let reader = run_log.reader();
            push_inputs(&mut run_log, "a", 10);

            let index = reader.index(RunLogIndexConfig::ByEffect {
                key: "a".to_string(),
                cursor: Cursor::Unique,
            });

            std::iter::from_fn(|| index.sample().map(|e| e.id)).collect()
        }

        assert_eq!(draw_all(42), draw_all(42));
    }

    #[test]
    fn unique_cursor_state_is_independent_per_index() {
        let mut run_log = SimpleEventRunLog::new(1);
        let reader = run_log.reader();
        push_inputs(&mut run_log, "a", 1);

        let index_a = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });
        let index_b = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });

        assert_eq!(index_a.sample().unwrap().id, 1);
        // A second, independent unique index over the same effect can still draw the same input.
        assert_eq!(index_b.sample().unwrap().id, 1);
    }
}
