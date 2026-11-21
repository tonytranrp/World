//! Lighting: the G-buffer to one lit colour target, at the trace's
//! (internal) resolution.
//!
//! Version 0 (`shade`) is a material colour times a fixed light per face.
//! Shadows, AO and GI (`shadow`, `ao`, `gi`) come later and add to the same
//! target. The voxel composite stretches the target over the viewport, so
//! per-pixel lighting work scales with the internal resolution, not the
//! window's (decision 0017).

pub mod shade;
