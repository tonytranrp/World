//! A fixed-order hash of parameters, so outputs can name the exact settings
//! that produced them. Integer mixing only; floats are hashed by bit pattern.

use crate::noise::hash::splitmix64;

/// Folds values into a 64-bit hash in the order they are written.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fingerprint(u64);

impl Fingerprint {
    pub fn new() -> Self {
        Self(0x5745_4E47_5445_5252)
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0 = splitmix64(self.0 ^ v);
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.u64(u64::from(v))
    }

    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.u64(u64::from(v.to_bits()) | 1 << 40)
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}
