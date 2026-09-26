use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct UniquePool<T> {
    effects: HashMap<String, Vec<T>>,
    cursors: HashMap<String, HashMap<String, Consumed>>,
}

impl<T> Default for UniquePool<T> {
    fn default() -> Self {
        UniquePool {
            effects: HashMap::new(),
            cursors: HashMap::new(),
        }
    }
}

impl<T: Clone> UniquePool<T> {
    pub fn push(&mut self, effect: &str, item: T) {
        match self.effects.get_mut(effect) {
            Some(items) => items.push(item),
            None => {
                self.effects.insert(effect.to_string(), vec![item]);
            }
        }
    }

    pub fn items(&self, effect: &str) -> &[T] {
        self.effects.get(effect).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn remaining(&self, effect: &str, cursor: &str) -> usize {
        let consumed = self
            .cursors
            .get(effect)
            .and_then(|cursors| cursors.get(cursor))
            .map_or(0, |consumed| consumed.total);
        self.items(effect).len() - consumed
    }

    pub fn take(&mut self, effect: &str, cursor: &str, index: usize) -> T {
        let len = self.items(effect).len();
        let consumed = self.consumed(effect, cursor);
        let position = consumed.nth_unconsumed(index, len);
        consumed.mark(position, len);
        self.effects[effect][position].clone()
    }

    pub fn mark(&mut self, effect: &str, cursor: &str, position: usize) {
        let len = self.items(effect).len();
        self.consumed(effect, cursor).mark(position, len);
    }

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

fn lowbit(i: usize) -> usize {
    i & i.wrapping_neg()
}

impl Consumed {
    fn capacity(&self) -> usize {
        self.tree.len() - 1
    }

    fn grow(&mut self, len: usize) {
        while self.capacity() < len {
            let capacity = self.capacity() * 2;
            self.tree.resize(capacity + 1, 0);
            self.tree[capacity] = self.total;
        }
    }

    fn mark(&mut self, position: usize, len: usize) {
        self.grow(len);
        let mut i = position + 1;
        while i <= self.capacity() {
            self.tree[i] += 1;
            i += lowbit(i);
        }
        self.total += 1;
    }

    fn nth_unconsumed(&mut self, index: usize, len: usize) -> usize {
        self.grow(len);
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
        position
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
    fn take_matches_naive_selection_as_the_pool_grows() {
        let mut pool = UniquePool::default();
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
            assert_eq!(pool.take("a", "c", index), expected as u64);
            consumed[expected] = true;
        }
    }

    #[test]
    fn cursors_and_effects_are_independent() {
        let mut pool = UniquePool::default();
        pool.push("a", 1);
        pool.push("b", 2);

        assert_eq!(pool.take("a", "x", 0), 1);
        assert_eq!(pool.remaining("a", "x"), 0);
        assert_eq!(pool.remaining("a", "y"), 1);
        assert_eq!(pool.remaining("b", "x"), 1);
        assert_eq!(pool.remaining("missing", "x"), 0);
    }

    #[test]
    fn mark_excludes_a_position_from_later_takes() {
        let mut pool = UniquePool::default();
        for id in 0..5 {
            pool.push("a", id);
        }
        pool.mark("a", "c", 0);

        assert_eq!(pool.remaining("a", "c"), 4);
        assert_eq!(pool.take("a", "c", 0), 1);
    }
}
