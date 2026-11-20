//! Deterministic noise and hashing: the base every stage builds on.
//!
//! - [`hash`]: SplitMix64 mixing, FNV-1a stage tags, per-column hashes and
//!   random streams, all integer arithmetic.
//! - [`perlin`]: Ken Perlin's improved noise (2002), ported to plain `f32`.
//! - [`fractal`]: fBm and ridged sums of seeded octaves, in 2D and 3D.
//! - [`curve`]: smoothstep, bilinear interpolation, piecewise-linear maps.
//!
//! Decision 0008's rules hold throughout: plain `f32` add, subtract, multiply,
//! divide, `floor`, `abs`, `min`/`max`/`clamp` (all exact or correctly
//! rounded IEEE operations), a fixed order of operations, no `mul_add`, no
//! platform math library (`sin`, `exp`, `powf`, ...), no runtime SIMD dispatch.

pub mod curve;
pub mod fractal;
pub mod hash;
pub mod perlin;

pub use curve::{PiecewiseLinear, bilerp, smoothstep};
pub use fractal::{Fractal, Fractal3, FractalParams, MAX_OCTAVES};
pub use hash::{Stream, derive, hash_column, splitmix64};
pub use perlin::Perlin;
