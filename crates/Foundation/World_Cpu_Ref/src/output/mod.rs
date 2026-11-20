//! Results of reference rendering: the G-buffer, a shaded image, PNG
//! encoding and exact comparison.
//!
//! - [`GBuffer`]: one [`TraceHit`](crate::trace::TraceHit) per pixel, compared
//!   exactly (every field, `t` by bit pattern), with a stable content hash.
//! - [`shade`]: material colour times a fixed per-face brightness; misses are
//!   [`SKY`]. Pure `f32` arithmetic, so images are identical byte for byte.
//! - [`Image`]: 8-bit RGBA with deterministic PNG encoding (`png` crate).

mod gbuffer;
mod image;
mod shade;

pub use gbuffer::{GBuffer, GBufferMismatch};
pub use image::{Image, OutputError};
pub use shade::{SKY, UNKNOWN_MATERIAL, face_brightness, shade};
