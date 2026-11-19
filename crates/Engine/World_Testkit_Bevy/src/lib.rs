//! World_Testkit_Bevy: Bevy test support. Dev-dependency only.
//!
//! - [`app`]: a headless `App` you can step frame by frame
//! - [`schedule`]: system-order randomization with a seed, to expose hidden ordering assumptions
//! - [`render`]: offscreen rendering on a chosen adapter, captured as an RGBA8 image

pub mod app;
pub mod render;
pub mod schedule;
