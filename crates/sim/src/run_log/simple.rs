use crate::run_log::{Metadata, RunLogReader, RunLogWriter};
use crate::{Input, Output};
use rand::RngExt;
use rand_pcg::Pcg32;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// An in-memory run log, used as the default when a [`crate::Simulation`] isn't given an on-disk
/// one.
#[derive(Debug, Default)]
pub struct SimpleEventRunLog {
    inputs: RefCell<Vec<Rc<Input>>>,
    outputs: RefCell<Vec<Output>>,
    metadata: RefCell<Vec<Metadata>>,
    /// Ids already handed out per `unique_for_effect` cursor; empty until that cursor's first
    /// draw.
    returned: RefCell<HashMap<String, HashSet<u64>>>,
}

impl SimpleEventRunLog {
    /// Returns an `Rc` since every real consumer needs a shared handle to hand to both a
    /// [`crate::Simulation`] and (for the same run) an [`crate::Audit`] - constructing a bare
    /// value only to immediately wrap it is the common case, not the exception.
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }
}

impl RunLogReader for SimpleEventRunLog {
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

    fn random_for_effect(&self, key: &str, rng: &mut Pcg32) -> Option<Rc<Input>> {
        let inputs = self.inputs.borrow();
        let candidates = inputs
            .iter()
            .filter(|e| e.effect == key)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            None
        } else {
            let idx = rng.random_range(0..candidates.len());
            candidates.get(idx).cloned().cloned()
        }
    }

    fn unique_for_effect(&self, key: &str, cursor: &str, rng: &mut Pcg32) -> Option<Rc<Input>> {
        let inputs = self.inputs.borrow();
        let mut returned = self.returned.borrow_mut();
        let returned = returned.entry(cursor.to_string()).or_default();

        let candidates = inputs
            .iter()
            .filter(|e| e.effect == key && !returned.contains(&e.id))
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            None
        } else {
            let idx = rng.random_range(0..candidates.len());
            let chosen = candidates.get(idx).cloned().cloned();
            if let Some(chosen) = &chosen {
                returned.insert(chosen.id);
            }
            chosen
        }
    }
}

impl RunLogWriter for SimpleEventRunLog {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rand_seeder::Seeder;

    fn rng(seed: u64) -> Pcg32 {
        Seeder::from(&format!("{seed}-run_log")).into_rng()
    }

    /// Builds a `SimpleEventRunLog`, populates it with ten inputs on effect "a", then draws
    /// `random_for_effect` `draws` times under an rng seeded from `seed`, returning the sampled
    /// ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let run_log = SimpleEventRunLog::new();
        let mut rng = rng(seed);

        for i in 1..=10u64 {
            run_log.push_input(Input {
                id: i,
                effect: "a".to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        (0..draws)
            .map(|_| run_log.random_for_effect("a", &mut rng).unwrap().id)
            .collect()
    }

    #[test]
    fn random_for_effect_is_deterministic_for_a_fixed_seed() {
        assert_eq!(sampled_ids(42, 5), sampled_ids(42, 5));
    }

    #[test]
    fn random_for_effect_differs_across_seeds() {
        assert_ne!(sampled_ids(1, 5), sampled_ids(2, 5));
    }

    #[test]
    fn random_for_effect_returns_none_when_no_matching_effect() {
        let run_log = SimpleEventRunLog::new();
        let mut rng = rng(1);

        assert!(run_log.random_for_effect("nonexistent", &mut rng).is_none());
    }

    fn push_inputs(run_log: &SimpleEventRunLog, effect: &str, count: u64) {
        for i in 1..=count {
            run_log.push_input(Input {
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
        push_inputs(&run_log, "a", 5);
        let mut rng = rng(1);

        let mut seen = HashSet::new();
        for _ in 0..5 {
            let sampled = run_log.unique_for_effect("a", "cursor", &mut rng).unwrap();
            assert!(
                seen.insert(sampled.id),
                "id {} returned more than once",
                sampled.id
            );
        }

        assert!(run_log.unique_for_effect("a", "cursor", &mut rng).is_none());
    }

    #[test]
    fn unique_cursor_is_deterministic_for_a_fixed_seed() {
        fn draw_all(seed: u64) -> Vec<u64> {
            let run_log = SimpleEventRunLog::new();
            push_inputs(&run_log, "a", 10);
            let mut rng = rng(seed);

            std::iter::from_fn(|| {
                run_log
                    .unique_for_effect("a", "cursor", &mut rng)
                    .map(|e| e.id)
            })
            .collect()
        }

        assert_eq!(draw_all(42), draw_all(42));
    }

    #[test]
    fn unique_cursor_state_is_independent_per_cursor() {
        let run_log = SimpleEventRunLog::new();
        push_inputs(&run_log, "a", 1);
        let mut rng = rng(1);

        assert_eq!(
            run_log
                .unique_for_effect("a", "cursor-a", &mut rng)
                .unwrap()
                .id,
            1
        );
        // A second, independent cursor over the same effect can still draw the same input.
        assert_eq!(
            run_log
                .unique_for_effect("a", "cursor-b", &mut rng)
                .unwrap()
                .id,
            1
        );
    }

    #[test]
    fn reflects_inputs_pushed_after_construction() {
        let run_log = SimpleEventRunLog::new();

        assert!(run_log.last().is_none());

        push_inputs(&run_log, "a", 1);

        assert_eq!(run_log.last().unwrap().id, 1);
    }

    #[test]
    fn last_for_effect_returns_most_recent_matching_effect() {
        let run_log = SimpleEventRunLog::new();

        for (i, effect) in [(1, "a"), (2, "b"), (3, "a")] {
            run_log.push_input(Input {
                id: i,
                effect: effect.to_string(),
                offset: i,
                timestamp: Utc::now().fixed_offset(),
                data: serde_json::json!(i),
                metadata: vec![],
            });
        }

        assert_eq!(run_log.last_for_effect("a").unwrap().id, 3);
        assert_eq!(run_log.last_for_effect("b").unwrap().id, 2);
        assert!(run_log.last_for_effect("c").is_none());
    }
}
