//! xxh3-64 hashing with a fixed byte order.
//!
//! xxh3's output is frozen across versions and platforms, so recorded golden
//! hashes stay valid. Every multi-byte value is written little-endian and
//! floats are hashed by their bit pattern, so the same state hashes the same
//! on every machine.

use xxhash_rust::xxh3::{Xxh3, xxh3_64};

/// Hash of a byte slice.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    xxh3_64(bytes)
}

/// Streaming hasher for simulation state.
pub struct StateHasher(Xxh3);

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHasher {
    pub fn new() -> Self {
        Self(Xxh3::new())
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update(bytes);
        self
    }

    pub fn write_u8(&mut self, v: u8) -> &mut Self {
        self.write_bytes(&[v])
    }

    pub fn write_u16(&mut self, v: u16) -> &mut Self {
        self.write_bytes(&v.to_le_bytes())
    }

    pub fn write_u32(&mut self, v: u32) -> &mut Self {
        self.write_bytes(&v.to_le_bytes())
    }

    pub fn write_u64(&mut self, v: u64) -> &mut Self {
        self.write_bytes(&v.to_le_bytes())
    }

    pub fn write_i32(&mut self, v: i32) -> &mut Self {
        self.write_bytes(&v.to_le_bytes())
    }

    /// Floats are hashed by bit pattern, so `-0.0` and `0.0` differ and a
    /// changed last bit is caught.
    pub fn write_f32(&mut self, v: f32) -> &mut Self {
        self.write_u32(v.to_bits())
    }

    pub fn finish(&self) -> u64 {
        self.0.digest()
    }
}

/// State that can feed itself into a [`StateHasher`] in a defined order.
pub trait HashState {
    fn hash_state(&self, hasher: &mut StateHasher);

    fn state_hash(&self) -> u64 {
        let mut hasher = StateHasher::new();
        self.hash_state(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Published xxh3-64 value for empty input; guards against an accidental
    /// algorithm or seed change.
    #[test]
    fn empty_input_matches_the_reference_vector() {
        assert_eq!(hash_bytes(b""), 0x2D06_8005_38D3_94C2);
    }

    #[test]
    fn streaming_equals_one_shot() {
        let mut h = StateHasher::new();
        h.write_bytes(b"hello ").write_bytes(b"world");
        assert_eq!(h.finish(), hash_bytes(b"hello world"));
    }

    #[test]
    fn integers_are_little_endian() {
        let mut h = StateHasher::new();
        h.write_u32(0x0102_0304);
        assert_eq!(h.finish(), hash_bytes(&[4, 3, 2, 1]));
    }

    #[test]
    fn float_hash_sees_the_sign_of_zero() {
        let (mut a, mut b) = (StateHasher::new(), StateHasher::new());
        a.write_f32(0.0);
        b.write_f32(-0.0);
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn trait_default_hashes_through_the_hasher() {
        struct Counter(u32);
        impl HashState for Counter {
            fn hash_state(&self, h: &mut StateHasher) {
                h.write_u32(self.0);
            }
        }
        assert_eq!(Counter(7).state_hash(), hash_bytes(&7u32.to_le_bytes()));
    }
}
