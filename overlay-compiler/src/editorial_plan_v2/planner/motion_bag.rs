//! Seeded shuffle bag: the ONE no-repeat motion draw.
//!
//! ANIMRAND-06. The image and caption lanes used to rotate the catalog
//! (`index = (offset + ordinal) % len`): the seed changed only the starting
//! point, so two runs shared the same relative sequence and a short run could
//! not explore the pool. A deterministic Fisher-Yates permutation of the
//! certified pool, walked once per cycle, gives every member exactly once
//! before any repeat, while staying a pure function of the seed.

use sha2::{Digest, Sha256};

use super::seeded_index_in;

/// A deterministic, no-repeat bag over one certified motion pool.
pub(super) struct SeededMotionBag {
    pool: Vec<String>,
    order: Vec<usize>,
    cursor: usize,
    cycle: u64,
    seed: String,
}

impl SeededMotionBag {
    pub(super) fn new(pool: &[String], seed: &str) -> Self {
        let seed = seed.to_string();
        let order = shuffled_order(pool.len(), &seed, 0);
        Self {
            pool: pool.to_vec(),
            order,
            cursor: 0,
            cycle: 0,
            seed,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pool.is_empty()
    }

    /// Next entrance in the current permutation. When a cycle is exhausted the
    /// bag reshuffles with a cycle-scoped seed, so the second pass is not a
    /// verbatim replay of the first.
    pub(super) fn take(&mut self) -> Option<String> {
        if self.pool.is_empty() {
            return None;
        }
        if self.cursor >= self.order.len() {
            self.cycle += 1;
            self.order = shuffled_order(self.pool.len(), &self.seed, self.cycle);
            self.cursor = 0;
        }
        let index = self.order[self.cursor];
        self.cursor += 1;
        self.pool.get(index).cloned()
    }
}

/// Deterministic Fisher-Yates permutation. `Sha256(seed NUL cycle NUL i)` is
/// reduced with the shared unbiased sampler, exactly like every other seeded
/// draw in the planner.
fn shuffled_order(len: usize, seed: &str, cycle: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    for i in (1..len).rev() {
        let digest = Sha256::digest(format!("{seed}\x00shuffle\x00{cycle}\x00{i}"));
        let j = seeded_index_in(&digest, 0, i + 1).unwrap_or(0);
        order.swap(i, j);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::SeededMotionBag;

    fn pool() -> Vec<String> {
        (0..8).map(|index| format!("motion_{index}")).collect()
    }

    /// The bag exhausts the pool before repeating, and the second cycle is a
    /// new permutation rather than a verbatim replay of the first.
    #[test]
    fn no_repeat_until_the_pool_is_exhausted() {
        let mut bag = SeededMotionBag::new(&pool(), "seed-bag");
        let first: Vec<String> = (0..8).map(|_| bag.take().expect("draw")).collect();
        let unique: std::collections::BTreeSet<&String> = first.iter().collect();
        assert_eq!(8, unique.len(), "first cycle repeated: {first:?}");

        let second: Vec<String> = (0..8).map(|_| bag.take().expect("draw")).collect();
        let unique_second: std::collections::BTreeSet<&String> = second.iter().collect();
        assert_eq!(8, unique_second.len(), "second cycle repeated: {second:?}");
        assert_ne!(first, second, "a new cycle must reshuffle");
    }

    #[test]
    fn same_seed_replays_the_same_sequence() {
        let mut a = SeededMotionBag::new(&pool(), "seed-bag");
        let mut b = SeededMotionBag::new(&pool(), "seed-bag");
        for _ in 0..8 {
            assert_eq!(a.take(), b.take());
        }
    }

    #[test]
    fn a_new_seed_reshuffles_the_sequence() {
        let mut a = SeededMotionBag::new(&pool(), "seed-a");
        let mut b = SeededMotionBag::new(&pool(), "seed-b");
        let first: Vec<String> = (0..8).map(|_| a.take().expect("draw")).collect();
        let second: Vec<String> = (0..8).map(|_| b.take().expect("draw")).collect();
        assert_ne!(first, second, "two seeds must not produce one order");
    }
}
