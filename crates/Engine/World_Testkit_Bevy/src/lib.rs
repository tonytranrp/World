//! World_Testkit_Bevy: Bevy test support. Dev-dependency only.
//!
//! - [`assets`]: in-memory and temporary-folder asset roots for hot-reload tests
//! - [`app`]: a headless `App` you can step frame by frame
//! - [`schedule`]: system-order randomization with a seed, to expose hidden ordering assumptions
//! - [`render`]: offscreen rendering on a chosen adapter, captured as an RGBA8 image

pub mod app;
pub mod assets;
pub mod render;
pub mod schedule;
