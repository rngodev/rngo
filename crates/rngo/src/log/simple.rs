use crate::log::pool::InputPool;
use crate::log::{Metadata, RunLogReader, RunLogWriter};
use crate::{Input, Output, Rng, RunLogError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// An in-memory run log; it can't run SQL queries.
#[derive(Debug, Default)]
pub struct SimpleEventRunLog {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    inputs: Vec<Arc<Input>>,
    outputs: Vec<Output>,
    metadata: Vec<Metadata>,
    pool: InputPool<Arc<Input>>,
}

impl State {
    fn insert_input(&mut self, input: Input) {
        let input = Arc::new(input);
        self.pool.push(&input.effect, input.clone());
        self.inputs.push(input);
    }

    /// The input pool, with `effect` tracked.
    fn pool(&mut self, effect: &str) -> &mut InputPool<Arc<Input>> {
        if !self.pool.is_tracked(effect) {
            let items = self
                .inputs
                .iter()
                .filter(|input| input.effect == effect)
                .cloned()
                .collect();
            self.pool.track(effect, items);
        }
        &mut self.pool
    }
}

impl SimpleEventRunLog {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn insert_input(&self, input: Input) {
        self.state().insert_input(input);
    }
}

impl RunLogReader for SimpleEventRunLog {
    fn last(&self) -> Option<Arc<Input>> {
        self.state().inputs.last().cloned()
    }

    fn last_for_effect(&self, key: &str) -> Option<Arc<Input>> {
        self.state().pool(key).last(key)
    }

    fn random_for_effect(&self, key: &str, rng: &mut Rng) -> Option<Arc<Input>> {
        self.state().pool(key).random(key, rng)
    }

    fn unique_for_effect(&self, key: &str, cursor: &str, rng: &mut Rng) -> Option<Arc<Input>> {
        self.state().pool(key).take(key, cursor, rng)
    }

    fn query(&self, _query: &str) -> Result<serde_json::Value, RunLogError> {
        Err(RunLogError::Unsupported)
    }
}

impl RunLogWriter for SimpleEventRunLog {
    fn push_input(&self, input: Input) -> Result<(), RunLogError> {
        self.insert_input(input);
        Ok(())
    }

    fn push_output(&self, output: Output) -> Result<(), RunLogError> {
        self.state().outputs.push(output);
        Ok(())
    }

    fn push_metadata(&self, metadata: Metadata) -> Result<(), RunLogError> {
        self.state().metadata.push(metadata);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::Timestamp;

    fn rng(seed: u64) -> Rng {
        Rng::from_seed(&format!("{seed}-run_log"))
    }

    /// Builds a `SimpleEventRunLog`, populates it with ten inputs on effect "a", then draws
    /// `random_for_effect` `draws` times under an rng seeded from `seed`, returning the sampled
    /// ids.
    fn sampled_ids(seed: u64, draws: usize) -> Vec<u64> {
        let run_log = SimpleEventRunLog::new();
        let mut rng = rng(seed);

        for i in 1..=10u64 {
            run_log
                .push_input(Input {
                    id: i,
                    effect: "a".to_string(),
                    timestamp: Timestamp::now(),
                    data: serde_json::json!(i),
                    metadata: vec![],
                })
                .unwrap();
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
            run_log
                .push_input(Input {
                    id: i,
                    effect: effect.to_string(),
                    timestamp: Timestamp::now(),
                    data: serde_json::json!(i),
                    metadata: vec![],
                })
                .unwrap();
        }
    }

    #[test]
    fn unique_cursor_never_repeats_and_exhausts() {
        let run_log = SimpleEventRunLog::new();
        push_inputs(&run_log, "a", 5);
        let mut rng = rng(1);

        let mut seen = std::collections::HashSet::new();
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
            run_log
                .push_input(Input {
                    id: i,
                    effect: effect.to_string(),
                    timestamp: Timestamp::now(),
                    data: serde_json::json!(i),
                    metadata: vec![],
                })
                .unwrap();
        }

        assert_eq!(run_log.last_for_effect("a").unwrap().id, 3);
        assert_eq!(run_log.last_for_effect("b").unwrap().id, 2);
        assert!(run_log.last_for_effect("c").is_none());
    }
}
