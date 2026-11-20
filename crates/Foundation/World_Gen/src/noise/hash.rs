//! Integer hashing and random streams.
//!
//! Everything here is integer arithmetic with wrapping operations, so it gives
//! the same bits on every platform. Seeds for the stages are derived by
//! hashing the world seed with a stage tag (domain separation), so streams of
//! different stages never correlate and no stage depends on what ran before it.

/// SplitMix64's output function (Steele, Lea and Flood, "Fast splittable
/// pseudorandom number generators", OOPSLA 2014): a bijective mix of 64 bits.
pub const fn splitmix64(x: u64) -> u64 {
    mix64(x.wrapping_add(GAMMA))
}

/// SplitMix64's golden gamma, the stream increment.
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// The finalizer of SplitMix64 (a variant of MurmurHash3's `fmix64`).
const fn mix64(x: u64) -> u64 {
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a over the bytes of a tag, so stage names can be used as domains.
pub const fn tag_hash(tag: &str) -> u64 {
    let bytes = tag.as_bytes();
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        i += 1;
    }
    hash
}

/// The seed of one domain (a stage or one of its noise fields) of a world
/// seed. Different tags give unrelated seeds.
pub const fn derive(seed: u64, tag: &str) -> u64 {
    splitmix64(seed ^ splitmix64(tag_hash(tag)))
}

/// A hash of a seed and an integer column, for per-column variation such as
/// layer thickness. Neighbouring columns get unrelated values.
pub const fn hash_column(seed: u64, x: i32, z: i32) -> u64 {
    let packed = (x as u32 as u64) | ((z as u32 as u64) << 32);
    splitmix64(seed ^ splitmix64(packed))
}

/// A SplitMix64 random stream: a counter advanced by the golden gamma and
/// mixed. Small, `Copy`, and never shared between stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stream {
    state: u64,
}

impl Stream {
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        mix64(self.state)
    }

    /// A uniform integer in `0..bound` (`bound` > 0). Uses a modulo, whose
    /// bias is irrelevant here and whose result is exact on every platform.
    pub fn below(&mut self, bound: u64) -> u64 {
        debug_assert!(bound > 0);
        self.next_u64() % bound.max(1)
    }

    /// A float in `[0, 1)` with 24 random bits, exactly representable.
    pub fn unit_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / 16_777_216.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_matches_the_reference_sequence() {
        // SplitMix64 seeded with 0: the first outputs of the reference C code
        // (state += gamma; z = mix(state)).
        let mut s = Stream::new(0);
        assert_eq!(s.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(s.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(s.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        assert_eq!(tag_hash(""), 0xCBF2_9CE4_8422_2325);
        assert_eq!(tag_hash("a"), 0xAF63_DC4C_8601_EC8C);
    }

    #[test]
    fn domains_and_columns_separate() {
        assert_ne!(derive(1, "a"), derive(1, "b"));
        assert_ne!(derive(1, "a"), derive(2, "a"));
        assert_ne!(hash_column(7, 1, 0), hash_column(7, 0, 1));
        assert_ne!(hash_column(7, -1, 0), hash_column(7, 1, 0));
    }

    #[test]
    fn unit_floats_stay_in_range() {
        let mut s = Stream::new(42);
        for _ in 0..1000 {
            let v = s.unit_f32();
            assert!((0.0..1.0).contains(&v));
        }
        assert!(s.below(5) < 5);
    }
}
