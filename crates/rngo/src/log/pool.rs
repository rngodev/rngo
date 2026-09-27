use rand::RngExt;
use rand_pcg::Pcg32;
use std::collections::HashMap;

/// In-memory index of each tracked effect's inputs, in push order, and of which ones each unique
/// cursor has consumed. Effects are only tracked once `track` is called for them, so effects that
/// are never looked up cost nothing.
#[derive(Debug)]
pub(crate) struct InputPool<T> {
    effects: HashMap<String, Vec<T>>,
    cursors: HashMap<String, HashMap<String, Consumed>>,
}

impl<T> Default for InputPool<T> {
    fn default() -> Self {
        InputPool {
            effects: HashMap::new(),
            cursors: HashMap::new(),
        }
    }
}

impl<T: Clone> InputPool<T> {
    /// Whether `track` has been called for `effect`.
    pub fn is_tracked(&self, effect: &str) -> bool {
        self.effects.contains_key(effect)
    }

    /// Starts tracking `effect`, seeded with its existing `items` in push order.
    pub fn track(&mut self, effect: &str, items: Vec<T>) {
        self.effects.insert(effect.to_string(), items);
    }

    /// Appends `item` to `effect`'s inputs; ignored if `effect` isn't tracked.
    pub fn push(&mut self, effect: &str, item: T) {
        if let Some(items) = self.effects.get_mut(effect) {
            items.push(item);
        }
    }

    /// `effect`'s most recently pushed input.
    pub fn last(&self, effect: &str) -> Option<T> {
        self.items(effect).last().cloned()
    }

    /// All of `effect`'s inputs in push order; empty if it has none or isn't tracked.
    fn items(&self, effect: &str) -> &[T] {
        self.effects.get(effect).map(Vec::as_slice).unwrap_or(&[])
    }

    /// A uniformly random input of `effect`, or `None` if it has none.
    pub fn random(&self, effect: &str, rng: &mut Pcg32) -> Option<T> {
        let items = self.items(effect);
        if items.is_empty() {
            return None;
        }
        let index = rng.random_range(0..items.len());
        Some(items[index].clone())
    }

    /// Consumes and returns a uniformly random input of `effect` that `cursor` hasn't consumed
    /// yet, or `None` if none remain.
    pub fn take(&mut self, effect: &str, cursor: &str, rng: &mut Pcg32) -> Option<T> {
        let remaining = self.remaining(effect, cursor);
        if remaining == 0 {
            return None;
        }
        let index = rng.random_range(0..remaining);
        Some(self.take_nth(effect, cursor, index))
    }

    /// Number of `effect`'s inputs that `cursor` hasn't consumed yet.
    fn remaining(&self, effect: &str, cursor: &str) -> usize {
        let consumed = self
            .cursors
            .get(effect)
            .and_then(|cursors| cursors.get(cursor))
            .map_or(0, |consumed| consumed.total);
        self.items(effect).len() - consumed
    }

    /// Consumes and returns the `index`th (0-based) input `cursor` hasn't consumed yet.
    /// `index` must be less than `remaining(effect, cursor)`.
    fn take_nth(&mut self, effect: &str, cursor: &str, index: usize) -> T {
        let consumed = self.consumed(effect, cursor);
        let position = consumed.nth_unconsumed(index);
        consumed.mark(position);
        self.effects[effect][position].clone()
    }

    /// `cursor`'s consumed set for `effect`, created empty on first use.
    fn consumed(&mut self, effect: &str, cursor: &str) -> &mut Consumed {
        if !self.cursors.contains_key(effect) {
            self.cursors.insert(effect.to_string(), HashMap::new());
        }
        let cursors = self.cursors.get_mut(effect).unwrap();
        if !cursors.contains_key(cursor) {
            cursors.insert(cursor.to_string(), Consumed::default());
        }
        cursors.get_mut(cursor).unwrap()
    }
}

impl<T: Clone + Ord> InputPool<T> {
    /// Records `item` as consumed by `cursor` without returning it; ignored if `item` isn't one of
    /// `effect`'s inputs. Used to replay prior draws when reopening a log. Assumes `effect`'s
    /// inputs were pushed in increasing order, and must not be called twice for the same item.
    pub fn mark(&mut self, effect: &str, cursor: &str, item: &T) {
        if let Ok(position) = self.items(effect).binary_search(item) {
            self.consumed(effect, cursor).mark(position);
        }
    }
}

/// Fenwick tree counting consumed positions. `tree` is 1-indexed (`tree[0]` is unused) and its
/// capacity is always a power of two; positions past the capacity are unconsumed. `total` is the
/// number of consumed positions.
#[derive(Debug)]
struct Consumed {
    tree: Vec<usize>,
    total: usize,
}

impl Default for Consumed {
    fn default() -> Self {
        Consumed {
            tree: vec![0, 0],
            total: 0,
        }
    }
}

/// Lowest set bit of `i`: the size of the range Fenwick node `i` covers.
fn lowbit(i: usize) -> usize {
    i.isolate_lowest_one()
}

impl Consumed {
    fn capacity(&self) -> usize {
        self.tree.len() - 1
    }

    /// Doubles capacity until it covers the 0-based `position`. New positions start unconsumed.
    fn grow(&mut self, position: usize) {
        while self.capacity() <= position {
            let capacity = self.capacity() * 2;
            self.tree.resize(capacity + 1, 0);
            self.tree[capacity] = self.total;
        }
    }

    /// Marks the 0-based `position` consumed.
    fn mark(&mut self, position: usize) {
        self.grow(position);
        let mut i = position + 1;
        while i <= self.capacity() {
            self.tree[i] += 1;
            i += lowbit(i);
        }
        self.total += 1;
    }

    /// 0-based position of the `index`th (0-based) unconsumed position.
    fn nth_unconsumed(&self, index: usize) -> usize {
        let mut position = 0;
        let mut rank = index + 1;
        let mut step = self.capacity();
        while step > 0 {
            let next = position + step;
            if next <= self.capacity() {
                let free = step - self.tree[next];
                if free < rank {
                    position = next;
                    rank -= free;
                }
            }
            step /= 2;
        }
        position + rank - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_nth(consumed: &[bool], index: usize) -> usize {
        consumed
            .iter()
            .enumerate()
            .filter(|(_, c)| !**c)
            .nth(index)
            .unwrap()
            .0
    }

    #[test]
    fn take_nth_matches_naive_selection_as_the_pool_grows() {
        let mut pool = InputPool::default();
        pool.track("a", vec![]);
        let mut consumed = vec![];
        let mut seed = 12345u64;

        for id in 0..2000u64 {
            pool.push("a", id);
            consumed.push(false);

            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            if !seed.is_multiple_of(3) {
                continue;
            }

            let remaining = pool.remaining("a", "c");
            assert_eq!(remaining, consumed.iter().filter(|c| !**c).count());
            if remaining == 0 {
                continue;
            }

            let index = (seed >> 33) as usize % remaining;
            let expected = naive_nth(&consumed, index);
            assert_eq!(pool.take_nth("a", "c", index), expected as u64);
            consumed[expected] = true;
        }
    }

    #[test]
    fn cursors_and_effects_are_independent() {
        let mut pool = InputPool::default();
        pool.track("a", vec![1]);
        pool.track("b", vec![2]);

        assert_eq!(pool.take_nth("a", "x", 0), 1);
        assert_eq!(pool.remaining("a", "x"), 0);
        assert_eq!(pool.remaining("a", "y"), 1);
        assert_eq!(pool.remaining("b", "x"), 1);
        assert_eq!(pool.remaining("missing", "x"), 0);
    }

    #[test]
    fn mark_excludes_an_item_from_later_takes() {
        let mut pool = InputPool::default();
        pool.track("a", vec![10, 20]);
        pool.push("a", 30);
        pool.mark("a", "c", &10);
        pool.mark("a", "c", &99);

        assert_eq!(pool.remaining("a", "c"), 2);
        assert_eq!(pool.take_nth("a", "c", 0), 20);
    }

    #[test]
    fn push_is_ignored_until_an_effect_is_tracked() {
        let mut pool = InputPool::default();
        pool.push("a", 1);
        assert!(!pool.is_tracked("a"));
        assert_eq!(pool.last("a"), None);

        pool.track("a", vec![2]);
        pool.push("a", 3);
        assert_eq!(pool.last("a"), Some(3));
        assert_eq!(pool.remaining("a", "c"), 2);
    }
}
