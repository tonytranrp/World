//! World_Cpu_Ref: a slow, exact CPU ray tracer.
//!
//! It is the ground truth the GPU tracer is compared against. It is written
//! independently of World_Core's own traversal so one bug cannot hide in both.
//!
//! The pipeline mirrors the GPU's primary pass: [`camera`] makes one ray per
//! pixel, [`trace`] walks it through any `world_core::VoxelSource` (the world
//! tree, a snapshot, a `DenseGrid`), and [`output`] records the hits in a
//! [`GBuffer`](output::GBuffer), shades them and writes PNGs.
//!
//! Conventions, defined once: positions are world voxels and the voxel at
//! `(x, y, z)` fills `[x, x + 1)` on each axis; axes are right-handed with
//! `+Y` up (see [`camera`]); hits are integer records (voxel, level, face,
//! material) plus a distance `t` in units of the ray direction's length (see
//! [`trace`]). Every floating-point operation is a plain IEEE operation in a
//! documented order: no fused multiply-add, no platform math library.
//!
//! ```
//! use world_core::{DenseGrid, MaterialId, VoxelPos};
//! use world_cpu_ref::camera::Ray;
//! use world_cpu_ref::trace::{Face, TraceOptions, trace};
//!
//! let mut grid = DenseGrid::new(2, VoxelPos::ORIGIN);
//! grid.set(VoxelPos::new(5, 3, 3), MaterialId(1));
//! let ray = Ray::new([0.5, 3.5, 3.5], [1.0, 0.0, 0.0]);
//! let hit = trace(&grid, &ray, &TraceOptions::default()).unwrap().unwrap();
//! assert_eq!((hit.voxel, hit.face, hit.t), (VoxelPos::new(5, 3, 3), Face::NegX, 4.5));
//! ```

pub mod camera;
pub mod output;
pub mod trace;

use world_core::VoxelSource;

use camera::Camera;
use output::GBuffer;
use trace::{TraceError, TraceOptions};

/// Traces every pixel of `camera` through `source`, row by row from the top,
/// into a G-buffer. Single-threaded: each pixel is independent and written to
/// its own slot, so the result never depends on scheduling.
pub fn render<S: VoxelSource + ?Sized>(
    source: &S,
    camera: &Camera,
    options: &TraceOptions,
) -> Result<GBuffer, TraceError> {
    let (w, h) = (camera.width(), camera.height());
    let mut hits = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            // Pixels inside the image always have a ray.
            let hit = match camera.ray(x, y) {
                Some(ray) => trace::trace(source, &ray, options)?,
                None => None,
            };
            hits.push(hit);
        }
    }
    match GBuffer::new(w, h, hits) {
        Some(buffer) => Ok(buffer),
        None => unreachable!("one hit was pushed per pixel"),
    }
}
