use rand::RngExt;
use rand_pcg::Pcg32;
use rand_seeder::Seeder;

/// A seeded random number generator; its draws are deterministic for a given seed and rngo version.
#[derive(Clone, Debug)]
pub struct Rng(pub(crate) Pcg32);

impl Rng {
    /// An RNG seeded by hashing `seed`.
    pub fn from_seed(seed: &str) -> Self {
        Rng(Seeder::from(seed).into_rng())
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0.random()
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0.random()
    }

    /// A uniform float in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        self.0.random()
    }

    /// A uniform index in `0..len`; panics if `len` is 0.
    pub fn index(&mut self, len: usize) -> usize {
        self.0.random_range(0..len)
    }
}
