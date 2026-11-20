//! Fractal sums of [`Perlin`] noise: fBm and ridged multifractal, over world
//! voxel coordinates.
//!
//! Each octave has its own seeded permutation and a random lattice offset, so
//! octaves never line up and the fractal's period is that of its lowest
//! octave (256 of its cells). Every octave's raw noise is clamped to
//! `[-1, 1]`, which makes the output bounds exact rather than empirical.

use serde::{Deserialize, Serialize};

use super::hash::{Stream, splitmix64};
use super::perlin::Perlin;
use crate::fingerprint::Fingerprint;
use world_core::consts::VOXELS_PER_METER;

/// The shape of one fractal noise field, as it appears in the terrain config.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FractalParams {
    /// Lattice cell size of the first (coarsest) octave, in metres.
    pub wavelength_m: f32,
    /// Number of octaves, 1 to [`MAX_OCTAVES`].
    pub octaves: u32,
    /// Frequency ratio between octaves, 1.5 to 4.
    pub lacunarity: f32,
    /// Amplitude ratio between octaves, above 0 and at most 1.
    pub persistence: f32,
}

/// The most octaves a field may have.
pub const MAX_OCTAVES: u32 = 12;

impl FractalParams {
    pub const fn new(wavelength_m: f32, octaves: u32) -> Self {
        Self {
            wavelength_m,
            octaves,
            lacunarity: 2.0,
            persistence: 0.5,
        }
    }

    /// The cell size of the finest octave, in metres, computed in `f64` (only
    /// used by validation, never by generation).
    pub fn finest_wavelength_m(&self) -> f64 {
        let mut w = f64::from(self.wavelength_m);
        for _ in 1..self.octaves {
            w /= f64::from(self.lacunarity);
        }
        w
    }

    /// Every problem with these parameters, prefixed by `name`, given the
    /// smallest finest-octave wavelength the field may have.
    pub fn problems(&self, name: &str, min_finest_m: f64) -> Vec<String> {
        let mut out = Vec::new();
        if !(self.wavelength_m.is_finite() && self.wavelength_m > 0.0) {
            out.push(format!(
                "{name}.wavelength_m {} must be above 0",
                self.wavelength_m
            ));
        }
        if !(1..=MAX_OCTAVES).contains(&self.octaves) {
            out.push(format!(
                "{name}.octaves {} must be 1 to {MAX_OCTAVES}",
                self.octaves
            ));
        }
        if !(1.5..=4.0).contains(&self.lacunarity) {
            out.push(format!(
                "{name}.lacunarity {} must be 1.5 to 4",
                self.lacunarity
            ));
        }
        if !(self.persistence > 0.0 && self.persistence <= 1.0) {
            out.push(format!(
                "{name}.persistence {} must be above 0 and at most 1",
                self.persistence
            ));
        }
        if out.is_empty() && self.finest_wavelength_m() < min_finest_m {
            out.push(format!(
                "{name}: the finest octave is {:.3} m, below the {min_finest_m} m its sampling lattice resolves",
                self.finest_wavelength_m()
            ));
        }
        out
    }

    pub(crate) fn fingerprint(&self, f: &mut Fingerprint) {
        f.f32(self.wavelength_m)
            .u32(self.octaves)
            .f32(self.lacunarity)
            .f32(self.persistence);
    }
}

/// One octave: a noise field, its frequency in cycles per voxel, its weight
/// and its lattice offset.
#[derive(Clone)]
struct Octave {
    noise: Perlin,
    frequency: f32,
    amplitude: f32,
    offset: [f32; 2],
}

/// A 2D fractal field over world voxel columns `(x, z)`.
#[derive(Clone)]
pub struct Fractal {
    octaves: Vec<Octave>,
    /// `1 / sum(amplitudes)`, so outputs stay within `[-1, 1]` (fBm) or
    /// `[0, 1]` (ridged).
    norm: f32,
}

impl Fractal {
    /// The field for validated `params`, seeded by `seed`.
    ///
    /// Octave `k` has frequency `f_k = f_(k-1) * lacunarity` starting from
    /// `f_0 = 1 / (wavelength_m * 64)` cycles per voxel, amplitude
    /// `a_k = a_(k-1) * persistence` from `a_0 = 1`, a permutation seeded by
    /// the `k`-th output of a SplitMix64 stream on `seed`, and an offset of two
    /// 24-bit fractions of 256 cells from the same stream.
    pub fn new(seed: u64, params: &FractalParams) -> Self {
        let mut stream = Stream::new(seed);
        let mut frequency = 1.0 / (params.wavelength_m * VOXELS_PER_METER as f32);
        let mut amplitude = 1.0f32;
        let mut sum = 0.0f32;
        let mut octaves = Vec::with_capacity(params.octaves as usize);
        for _ in 0..params.octaves {
            let noise = Perlin::from_seed(splitmix64(stream.next_u64()));
            let offset = [stream.unit_f32() * 256.0, stream.unit_f32() * 256.0];
            octaves.push(Octave {
                noise,
                frequency,
                amplitude,
                offset,
            });
            sum += amplitude;
            frequency *= params.lacunarity;
            amplitude *= params.persistence;
        }
        Self {
            octaves,
            norm: 1.0 / sum,
        }
    }

    /// The raw noise of one octave at a column, clamped to `[-1, 1]`: the
    /// input is `x * f + offset_x`, `z * f + offset_z`.
    #[inline]
    fn octave(o: &Octave, x: f32, z: f32) -> f32 {
        o.noise
            .noise2(x * o.frequency + o.offset[0], z * o.frequency + o.offset[1])
            .clamp(-1.0, 1.0)
    }

    /// Fractal Brownian motion: `norm * sum(a_k * n_k)`, summed from the
    /// coarsest octave up. In `[-1, 1]`.
    pub fn fbm(&self, x: f32, z: f32) -> f32 {
        let mut sum = 0.0f32;
        for o in &self.octaves {
            sum += o.amplitude * Self::octave(o, x, z);
        }
        sum * self.norm
    }

    /// Ridged noise: `norm * sum(a_k * (1 - |n_k|)²)`, sharp crests where an
    /// octave crosses zero. In `[0, 1]`.
    pub fn ridged(&self, x: f32, z: f32) -> f32 {
        let mut sum = 0.0f32;
        for o in &self.octaves {
            let r = 1.0 - Self::octave(o, x, z).abs();
            sum += o.amplitude * (r * r);
        }
        sum * self.norm
    }
}

/// One octave of a 3D field: a noise field, its frequencies in cycles per
/// voxel (the vertical one scaled), its weight and its lattice offset.
#[derive(Clone)]
struct Octave3 {
    noise: Perlin,
    frequency: f32,
    frequency_y: f32,
    amplitude: f32,
    offset: [f32; 3],
}

/// A 3D fractal field over world voxels `(x, y, z)`, for volumes such as
/// caves. Built like [`Fractal`], with a third offset per octave and the
/// vertical frequency multiplied by `y_scale` (above 1 flattens features).
#[derive(Clone)]
pub struct Fractal3 {
    octaves: Vec<Octave3>,
    norm: f32,
}

impl Fractal3 {
    /// The field for validated `params`, seeded by `seed`. Octave `k` takes
    /// the `k`-th permutation seed and three offset fractions from a
    /// SplitMix64 stream on `seed`; its vertical frequency is
    /// `f_k * y_scale`.
    pub fn new(seed: u64, params: &FractalParams, y_scale: f32) -> Self {
        let mut stream = Stream::new(seed);
        let mut frequency = 1.0 / (params.wavelength_m * VOXELS_PER_METER as f32);
        let mut amplitude = 1.0f32;
        let mut sum = 0.0f32;
        let mut octaves = Vec::with_capacity(params.octaves as usize);
        for _ in 0..params.octaves {
            let noise = Perlin::from_seed(splitmix64(stream.next_u64()));
            let offset = [
                stream.unit_f32() * 256.0,
                stream.unit_f32() * 256.0,
                stream.unit_f32() * 256.0,
            ];
            octaves.push(Octave3 {
                noise,
                frequency,
                frequency_y: frequency * y_scale,
                amplitude,
                offset,
            });
            sum += amplitude;
            frequency *= params.lacunarity;
            amplitude *= params.persistence;
        }
        Self {
            octaves,
            norm: 1.0 / sum,
        }
    }

    /// Fractal Brownian motion at a voxel position: `norm * sum(a_k * n_k)`
    /// with every octave clamped to `[-1, 1]`. In `[-1, 1]`.
    pub fn fbm(&self, x: f32, y: f32, z: f32) -> f32 {
        let mut sum = 0.0f32;
        for o in &self.octaves {
            let n = o
                .noise
                .noise3(
                    x * o.frequency + o.offset[0],
                    y * o.frequency_y + o.offset[1],
                    z * o.frequency + o.offset[2],
                )
                .clamp(-1.0, 1.0);
            sum += o.amplitude * n;
        }
        sum * self.norm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> FractalParams {
        FractalParams::new(16.0, 4)
    }

    #[test]
    fn outputs_stay_in_their_ranges() {
        let f = Fractal::new(3, &params());
        let mut s = Stream::new(5);
        for _ in 0..5_000 {
            let x = (s.next_u64() % 1_000_000) as f32 - 500_000.0;
            let z = (s.next_u64() % 1_000_000) as f32 - 500_000.0;
            let (a, b) = (f.fbm(x, z), f.ridged(x, z));
            assert!((-1.0..=1.0).contains(&a), "{a}");
            assert!((0.0..=1.0).contains(&b), "{b}");
        }
    }

    #[test]
    fn same_seed_same_field() {
        let (a, b) = (Fractal::new(3, &params()), Fractal::new(3, &params()));
        let c = Fractal::new(4, &params());
        assert_eq!(a.fbm(123.0, -77.0).to_bits(), b.fbm(123.0, -77.0).to_bits());
        assert_ne!(a.fbm(123.0, -77.0), c.fbm(123.0, -77.0));
    }

    #[test]
    fn validation_reports_every_problem() {
        let bad = FractalParams {
            wavelength_m: -1.0,
            octaves: 0,
            lacunarity: 9.0,
            persistence: 0.0,
        };
        assert_eq!(bad.problems("x", 0.0).len(), 4);
        assert!(params().problems("x", 1.0).is_empty());
        assert_eq!(params().problems("x", 4.0).len(), 1, "finest octave is 2 m");
    }

    #[test]
    fn the_3d_field_stays_in_range_and_is_seeded() {
        let f = Fractal3::new(3, &params(), 2.0);
        let mut s = Stream::new(8);
        for _ in 0..5_000 {
            let mut c = || (s.next_u64() % 1_000_000) as f32 - 500_000.0;
            let (x, y, z) = (c(), c(), c());
            assert!((-1.0..=1.0).contains(&f.fbm(x, y, z)));
        }
        let g = Fractal3::new(4, &params(), 2.0);
        assert_ne!(f.fbm(10.5, 3.25, -7.0), g.fbm(10.5, 3.25, -7.0));
    }
}
