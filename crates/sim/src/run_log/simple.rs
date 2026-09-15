use crate::run_log::{Cursor, Metadata, RunLogIndex, RunLogIndexConfig, RunLogReader};
use crate::{Input, Output, RunLog, RunLogWriter};
use rand::RngExt;
use rand_pcg::Pcg32;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Debug)]
pub struct SimpleEventRunLogReader {
    inputs: Rc<RefCell<Vec<Rc<Input>>>>,
}

impl RunLogReader for SimpleEventRunLogReader {
    fn last(&self) -> Option<Rc<Input>> {
        self.inputs.borrow().last().cloned()
    }

    fn last_for_effect(&self, key: &str) -> Option<Rc<Input>> {
        self.inputs
            .borrow()
            .iter()
            .rfind(|e| e.effect == key)
            .cloned()
    }

    // No SQL engine backs an in-memory run log.
    fn query(&self, _query: &str) -> Option<serde_json::Value> {
        None
    }

    fn index(&self, config: RunLogIndexConfig) -> Box<dyn RunLogIndex> {
        Box::new(SimpleEventRunLogIndex {
            inputs: Rc::clone(&self.inputs),
            returned: HashSet::new(),
            config,
        })
    }
}

/// An in-memory [`RunLog`], used as the default when a [`crate::Simulation`] isn't given an
/// on-disk one.
#[derive(Debug, Default)]
pub struct SimpleEventRunLog {
    inputs: Rc<RefCell<Vec<Rc<Input>>>>,
    outputs: Rc<RefCell<Vec<Output>>>,
    metadata: Rc<RefCell<Vec<Metadata>>>,
}

impl SimpleEventRunLog {
    pub fn new() -> Self {
        SimpleEventRunLog {
            inputs: Rc::new(RefCell::new(Vec::new())),
            outputs: Rc::new(RefCell::new(Vec::new())),
            metadata: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl RunLog for SimpleEventRunLog {
    fn reader(&self) -> Rc<dyn RunLogReader> {
        Rc::new(SimpleEventRunLogReader {
            inputs: Rc::clone(&self.inputs),
        })
    }

    fn writer(&self) -> Rc<dyn RunLogWriter> {
        Rc::new(SimpleEventRunLogWriter {
            inputs: Rc::clone(&self.inputs),
            outputs: Rc::clone(&self.outputs),
            metadata: Rc::clone(&self.metadata),
        })
    }
}

#[derive(Debug)]
struct SimpleEventRunLogWriter {
    inputs: Rc<RefCell<Vec<Rc<Input>>>>,
    outputs: Rc<RefCell<Vec<Output>>>,
    metadata: Rc<RefCell<Vec<Metadata>>>,
}

impl RunLogWriter for SimpleEventRunLogWriter {
    fn push_input(&self, input: Input) {
        self.inputs.borrow_mut().push(Rc::new(input));
    }

    fn push_output(&self, output: Output) {
        self.outputs.borrow_mut().push(output);
    }

    fn push_metadata(&self, metadata: Metadata) {
        self.metadata.borrow_mut().push(metadata);
    }
}

#[derive(Debug)]
pub struct SimpleEventRunLogIndex {
    inputs: Rc<RefCell<Vec<Rc<Input>>>>,
    /// Ids already handed out by this index under [`Cursor::Unique`]; empty and unused otherwise.
    returned: HashSet<u64>,
    config: RunLogIndexConfig,
}

impl RunLogIndex for SimpleEventRunLogIndex {
    fn sample(&mut self, rng: &mut Pcg32) -> Option<Rc<Input>> {
        let inputs = self.inputs.borrow();

        let RunLogIndexConfig::ByEffect { key, cursor } = &self.config;

        let filtered_events = inputs.iter().filter(|e| &e.effect == key);

        match cursor {
            Cursor::Random => {
                let filtered_events = filtered_events.collect::<Vec<_>>();
                if filtered_events.is_empty() {
                    None
                } else {
                    let idx = rng.random_range(0..filtered_events.len());
                    filtered_events.get(idx).cloned().cloned()
                }
            }
            Cursor::Unique => {
                let candidates = filtered_events
                    .filter(|e| !self.returned.contains(&e.id))
                    .collect::<Vec<_>>();

                if candidates.is_empty() {
                    None
                } else {
                    let idx = rng.random_range(0..candidates.len());
                    let chosen = candidates.get(idx).cloned().cloned();
                    if let Some(chosen) = &chosen {
                        self.returned.insert(chosen.id);
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
    use rand_seeder::Seeder;

    fn rng(seed: u64) -> Pcg32 {
        Seeder::from(&format!("{seed}-run_log")).into_rng()
    }

    /// Builds a `SimpleEventRunLog` under the given seed, populates it with ten inputs on effect
    /// "a", then samples that effect's index `draws` times, returning the sampled ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let run_log = SimpleEventRunLog::new();
        let reader = run_log.reader();
        let writer = run_log.writer();
        let mut rng = rng(seed);

        for i in 1..=10u64 {
            writer.push_input(Input {
                id: i,
                effect: "a".to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        let mut index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Random,
        });

        (0..draws)
            .map(|_| index.sample(&mut rng).unwrap().id)
            .collect()
    }

    #[test]
    fn index_sample_is_deterministic_for_a_fixed_seed() {
        assert_eq!(sampled_ids(42, 5), sampled_ids(42, 5));
    }

    #[test]
    fn index_sample_differs_across_seeds() {
        assert_ne!(sampled_ids(1, 5), sampled_ids(2, 5));
    }

    fn push_inputs(run_log: &SimpleEventRunLog, effect: &str, count: u64) {
        let writer = run_log.writer();
        for i in 1..=count {
            writer.push_input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }
    }

    #[test]
    fn unique_cursor_never_repeats_and_exhausts() {
        let run_log = SimpleEventRunLog::new();
        let reader = run_log.reader();
        push_inputs(&run_log, "a", 5);
        let mut rng = rng(1);

        let mut index = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });

        let mut seen = HashSet::new();
        for _ in 0..5 {
            let sampled = index.sample(&mut rng).unwrap();
            assert!(
                seen.insert(sampled.id),
                "id {} returned more than once",
                sampled.id
            );
        }

        assert!(index.sample(&mut rng).is_none());
    }

    #[test]
    fn unique_cursor_is_deterministic_for_a_fixed_seed() {
        fn draw_all(seed: u64) -> Vec<u64> {
            let run_log = SimpleEventRunLog::new();
            let reader = run_log.reader();
            push_inputs(&run_log, "a", 10);
            let mut rng = rng(seed);

            let mut index = reader.index(RunLogIndexConfig::ByEffect {
                key: "a".to_string(),
                cursor: Cursor::Unique,
            });

            std::iter::from_fn(|| index.sample(&mut rng).map(|e| e.id)).collect()
        }

        assert_eq!(draw_all(42), draw_all(42));
    }

    #[test]
    fn unique_cursor_state_is_independent_per_index() {
        let run_log = SimpleEventRunLog::new();
        let reader = run_log.reader();
        push_inputs(&run_log, "a", 1);
        let mut rng = rng(1);

        let mut index_a = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });
        let mut index_b = reader.index(RunLogIndexConfig::ByEffect {
            key: "a".to_string(),
            cursor: Cursor::Unique,
        });

        assert_eq!(index_a.sample(&mut rng).unwrap().id, 1);
        // A second, independent unique index over the same effect can still draw the same input.
        assert_eq!(index_b.sample(&mut rng).unwrap().id, 1);
    }

    #[test]
    fn reader_reflects_inputs_pushed_after_it_was_created() {
        let run_log = SimpleEventRunLog::new();
        let reader = run_log.reader();

        assert!(reader.last().is_none());

        push_inputs(&run_log, "a", 1);

        assert_eq!(reader.last().unwrap().id, 1);
    }

    #[test]
    fn last_for_effect_returns_most_recent_matching_effect() {
        let run_log = SimpleEventRunLog::new();
        let reader = run_log.reader();
        let writer = run_log.writer();

        for (i, effect) in [(1, "a"), (2, "b"), (3, "a")] {
            writer.push_input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        assert_eq!(reader.last_for_effect("a").unwrap().id, 3);
        assert_eq!(reader.last_for_effect("b").unwrap().id, 2);
        assert!(reader.last_for_effect("c").is_none());
    }
}
