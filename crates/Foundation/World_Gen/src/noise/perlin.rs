//! Perlin's improved gradient noise, ported to plain `f32`.
//!
//! Source algorithm: Ken Perlin, "Improving Noise", SIGGRAPH 2002, and his
//! reference Java implementation `ImprovedNoise` (<https://mrl.cs.nyu.edu/~perlin/noise/>).
//! The port keeps the reference's structure and order of operations exactly:
//! the quintic fade `t³(t(6t - 15) + 10)`, `lerp(t, a, b) = a + t(b - a)`,
//! the 12 edge gradients picked by the low four bits of the hash, and the
//! doubled 256-entry permutation. With [`Perlin::reference`] it reproduces the
//! reference's published value `noise(3.14, 42, 7) = 0.136919958784` to f32
//! precision; seeded instances shuffle the permutation with SplitMix64.
//!
//! Determinism (decision 0008): only `+ - *` on `f32`, `f32::floor` (exact on
//! every IEEE platform), integer hashing, and no fused multiply-add (Rust never
//! contracts `a * b + c` unless `mul_add` is called).
//!
//! The pattern repeats every 256 lattice cells on each axis, like the
//! reference. Fractal sums give every octave its own permutation and offset.

use super::hash::Stream;

/// Ken Perlin's reference permutation of `0..=255`.
pub const REFERENCE_PERMUTATION: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

/// One improved-noise field: a permutation, doubled so lookups never wrap.
#[derive(Clone)]
pub struct Perlin {
    perm: [u8; 512],
}

impl Perlin {
    /// The reference field, with Perlin's own permutation.
    pub fn reference() -> Self {
        Self::from_permutation(&REFERENCE_PERMUTATION)
    }

    /// A field whose permutation is `0..=255` shuffled by Fisher-Yates, driven
    /// by a SplitMix64 stream from `seed`.
    pub fn from_seed(seed: u64) -> Self {
        let mut p = [0u8; 256];
        for (i, v) in p.iter_mut().enumerate() {
            *v = i as u8;
        }
        let mut stream = Stream::new(seed);
        for i in (1..256usize).rev() {
            let j = stream.below(i as u64 + 1) as usize;
            p.swap(i, j);
        }
        Self::from_permutation(&p)
    }

    fn from_permutation(p: &[u8; 256]) -> Self {
        let mut perm = [0u8; 512];
        for i in 0..512 {
            perm[i] = p[i & 255];
        }
        Self { perm }
    }

    fn p(&self, i: usize) -> usize {
        self.perm[i] as usize
    }

    /// 3D improved noise, in about `[-1, 1]`, zero at every lattice point.
    pub fn noise3(&self, x: f32, y: f32, z: f32) -> f32 {
        let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
        let (xi, yi, zi) = (lattice(xf), lattice(yf), lattice(zf));
        let (x, y, z) = (x - xf, y - yf, z - zf);
        let (u, v, w) = (fade(x), fade(y), fade(z));
        let a = self.p(xi) + yi;
        let aa = self.p(a) + zi;
        let ab = self.p(a + 1) + zi;
        let b = self.p(xi + 1) + yi;
        let ba = self.p(b) + zi;
        let bb = self.p(b + 1) + zi;
        let (x1, y1, z1) = (x - 1.0, y - 1.0, z - 1.0);
        lerp(
            w,
            lerp(
                v,
                lerp(u, grad(self.p(aa), x, y, z), grad(self.p(ba), x1, y, z)),
                lerp(u, grad(self.p(ab), x, y1, z), grad(self.p(bb), x1, y1, z)),
            ),
            lerp(
                v,
                lerp(
                    u,
                    grad(self.p(aa + 1), x, y, z1),
                    grad(self.p(ba + 1), x1, y, z1),
                ),
                lerp(
                    u,
                    grad(self.p(ab + 1), x, y1, z1),
                    grad(self.p(bb + 1), x1, y1, z1),
                ),
            ),
        )
    }

    /// 2D noise: the `z = 0` slice of [`noise3`](Self::noise3), evaluated with
    /// only its four `z = 0` corners. Equal to `noise3(x, y, 0.0)` (compared
    /// with `==`, so the sign of a zero may differ).
    pub fn noise2(&self, x: f32, y: f32) -> f32 {
        let (xf, yf) = (x.floor(), y.floor());
        let (xi, yi) = (lattice(xf), lattice(yf));
        let (x, y) = (x - xf, y - yf);
        let (u, v) = (fade(x), fade(y));
        let a = self.p(xi) + yi;
        let b = self.p(xi + 1) + yi;
        let (aa, ab, ba, bb) = (self.p(a), self.p(a + 1), self.p(b), self.p(b + 1));
        let (x1, y1) = (x - 1.0, y - 1.0);
        lerp(
            v,
            lerp(u, grad(self.p(aa), x, y, 0.0), grad(self.p(ba), x1, y, 0.0)),
            lerp(
                u,
                grad(self.p(ab), x, y1, 0.0),
                grad(self.p(bb), x1, y1, 0.0),
            ),
        )
    }
}

/// The lattice index modulo 256 of a floored coordinate. Saturating
/// float-to-int conversion keeps huge or non-finite inputs panic-free.
fn lattice(floored: f32) -> usize {
    (floored as i32 & 255) as usize
}

/// Perlin's quintic fade, `t³(t(6t - 15) + 10)`, in the reference's order.
pub fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// `a + t(b - a)`, the reference's lerp.
pub fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// The reference gradient: twelve cube-edge directions (four repeated),
/// chosen by the low four bits of the hash.
fn grad(hash: usize, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_permutation_is_a_permutation() {
        let mut seen = [false; 256];
        for &v in &REFERENCE_PERMUTATION {
            assert!(!seen[v as usize], "{v} twice");
            seen[v as usize] = true;
        }
    }

    #[test]
    fn matches_the_published_reference_value() {
        // ImprovedNoise.noise(3.14, 42, 7) in double precision; 3.14 is the
        // reference's input, not an approximation of pi.
        #[allow(clippy::approx_constant)]
        let v = Perlin::reference().noise3(3.14, 42.0, 7.0);
        assert!(
            (f64::from(v) - 0.136_919_958_784_000_12).abs() < 1e-6,
            "{v}"
        );
    }

    #[test]
    fn noise_vanishes_on_the_lattice_and_stays_bounded() {
        let n = Perlin::from_seed(9);
        assert_eq!(n.noise3(5.0, -3.0, 17.0), 0.0);
        assert_eq!(n.noise2(-8.0, 300.0), 0.0);
        let mut s = Stream::new(1);
        for _ in 0..10_000 {
            let x = s.unit_f32() * 512.0 - 256.0;
            let y = s.unit_f32() * 512.0 - 256.0;
            let v = n.noise2(x, y);
            assert!(v.abs() <= 1.1, "{v} at ({x}, {y})");
            assert_eq!(v, n.noise3(x, y, 0.0));
        }
    }

    #[test]
    fn seeded_permutations_differ_and_are_permutations() {
        let a = Perlin::from_seed(1);
        let b = Perlin::from_seed(2);
        assert_ne!(a.perm[..256], b.perm[..256]);
        let mut sorted = a.perm[..256].to_vec();
        sorted.sort_unstable();
        assert!(sorted.iter().enumerate().all(|(i, &v)| v as usize == i));
    }

    #[test]
    fn non_finite_inputs_do_not_panic() {
        let n = Perlin::reference();
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 3.0e38] {
            let _ = n.noise2(v, 1.5);
            let _ = n.noise3(1.5, v, 0.5);
        }
    }
}
