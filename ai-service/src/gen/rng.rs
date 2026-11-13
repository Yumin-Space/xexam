//! SplitMix64: the only source of randomness of the planner. The seed is an input (the job key), so a job key always
//! gives the same plan.

/// SplitMix64 generator.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng { state: seed }
    }

    /// The seed of a job: the first 16 hex digits of the job key as a big-endian u64 (a shorter or malformed key is
    /// read as far as it is valid hex, 0 when nothing is).
    pub fn seed_from_key(key: &str) -> u64 {
        let head: String = key.chars().take(16).collect();
        u64::from_str_radix(&head, 16).unwrap_or(0)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (`n > 0`), by `next_u64() % n`.
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        (self.next_u64() % n as u64) as usize
    }

    /// Fisher-Yates from the last index down with `next_u64() % (i + 1)`.
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next_u64() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_reference_vector() {
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(r.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn seed_from_key_reads_16_hex_digits() {
        assert_eq!(Rng::seed_from_key("00000000000000ff0123456789abcdef"), 0xff);
        assert_eq!(Rng::seed_from_key("ffffffffffffffff00"), u64::MAX);
        assert_eq!(Rng::seed_from_key("zz"), 0);
        assert_eq!(Rng::seed_from_key("1f"), 0x1f);
    }

    #[test]
    fn shuffle_is_a_deterministic_permutation() {
        let mut a: Vec<usize> = (0..20).collect();
        let mut b = a.clone();
        Rng::new(7).shuffle(&mut a);
        Rng::new(7).shuffle(&mut b);
        assert_eq!(a, b);
        let mut sorted = a.clone();
        sorted.sort();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>());
        assert_ne!(a, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn different_seeds_differ_and_small_slices_work() {
        let mut a: Vec<usize> = (0..20).collect();
        let mut b = a.clone();
        Rng::new(1).shuffle(&mut a);
        Rng::new(2).shuffle(&mut b);
        assert_ne!(a, b);
        let mut empty: Vec<u8> = vec![];
        Rng::new(1).shuffle(&mut empty);
        let mut one = vec![9];
        Rng::new(1).shuffle(&mut one);
        assert_eq!(one, vec![9]);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::new(42);
        for n in 1..30 {
            assert!(r.below(n) < n);
        }
    }
}
