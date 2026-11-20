//! Ray traversal of any [`VoxelSource`], with exact integer hit decisions.
//!
//! The algorithm is the 64-tree traversal of dubiousconst282's guide
//! (<https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing/>):
//!
//! - **Octant mirroring.** Each axis with a negative direction is mirrored
//!   (`v -> size - 1 - v` on cube-local voxel coordinates), so the walk only
//!   ever steps in `+` directions. Child slots are read through the mirror
//!   (`local = v ^ (size - 1)` inside the cube).
//! - **Mask skipping.** Descending stops at the first child whose mask bit is
//!   clear; the walk then leaves that whole empty cell (`4^level` voxels per
//!   axis) in one step.
//! - **Ancestor stack.** After a step, the nodes that still contain the new
//!   position are kept: the highest set bit of `old ^ new` over the three
//!   axes names the lowest common ancestor, and descent resumes there.
//!
//! **Exactness.** Every decision is an integer comparison of voxel
//! coordinates or a comparison of crossing distances, and every distance is
//! computed the same way as World_Testkit's dense DDA: the crossing of
//! integer plane `b` on an axis is `(b as f32 - origin) / dir`, in the
//! original (unmirrored) space, never accumulated. Crossings are ordered by
//! `(t, axis)` with the lower axis first on a tie, which is the DDA's rule
//! for rays through edges and corners. Leaving an empty cell jumps every
//! other axis to exactly the voxel the DDA would be in at that moment (the
//! last crossing ordered before the exit, found by a monotone search over
//! those same distances). So the walk visits a subsequence of the DDA's
//! voxels containing every voxel that is not inside a skipped empty cell, and
//! the first solid voxel, its entry face and its `t` are bit-identical to the
//! DDA's, on the CPU. The GPU compares the integer fields exactly and `t`
//! with a tolerance (decision 0005).
//!
//! The start voxel is `floor(origin)` per axis, as in the DDA, so a ray lying
//! exactly on a voxel plane belongs to the voxel above it; a ray starting in
//! a solid voxel hits it with [`Face::Inside`] at `t = 0`. A ray from outside
//! the cube enters at the last of its near-plane crossings.
//!
//! **Stop level.** [`TraceOptions::stop_level`] `k` traces the occupancy of
//! cells `4^k` voxels wide: a cell is solid when its mask bit is set (decision
//! 0005, "a mip level is read as stop at level L; a set mask bit is solid";
//! with `k >= 1` the cell is a node of tree level `k - 1`). Its material is
//! the representative (see [`representative`]), and the hit's `voxel` is the
//! voxel where the ray entered the cell. `k = 0` is full resolution.

mod axis;
mod coarse;
mod walk;

pub use coarse::representative;

use std::fmt;

use world_core::{MaterialId, VoxelBox, VoxelPos, VoxelSource};

use crate::camera::{Ray, RayError};

/// The most levels a traced source may have: `4^15` voxels per axis keeps
/// every coordinate exact in `u32` slot math and `i64` stepping.
pub const MAX_LEVELS: u32 = 15;

/// How to trace.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceOptions {
    /// Trace cells `4^stop_level` voxels wide; 0 is full resolution. At most
    /// the source's level count (the whole cube as one cell).
    pub stop_level: u32,
    /// Hits farther than this (in units of the ray direction's length) are
    /// misses. Must not be NaN or negative.
    pub max_t: f32,
}

impl Default for TraceOptions {
    /// Full resolution, unlimited distance.
    fn default() -> Self {
        Self {
            stop_level: 0,
            max_t: f32::INFINITY,
        }
    }
}

/// The voxel face a ray entered through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    NegX,
    PosX,
    NegY,
    PosY,
    NegZ,
    PosZ,
    /// The ray started inside the solid cell.
    Inside,
}

impl Face {
    /// The face entered when crossing a plane of `axis` (0, 1, 2 for x, y,
    /// z) moving in `+` (`positive`) or `-`: moving `+` enters through `-`.
    pub const fn entered(axis: usize, positive: bool) -> Self {
        match (axis, positive) {
            (0, true) => Self::NegX,
            (0, false) => Self::PosX,
            (1, true) => Self::NegY,
            (1, false) => Self::PosY,
            (2, true) => Self::NegZ,
            _ => Self::PosZ,
        }
    }

    /// The axis of the face's normal, or `None` for `Inside`.
    pub const fn axis(self) -> Option<usize> {
        match self {
            Self::NegX | Self::PosX => Some(0),
            Self::NegY | Self::PosY => Some(1),
            Self::NegZ | Self::PosZ => Some(2),
            Self::Inside => None,
        }
    }

    /// The outward unit normal; zero for `Inside`.
    pub const fn normal(self) -> [i32; 3] {
        match self {
            Self::NegX => [-1, 0, 0],
            Self::PosX => [1, 0, 0],
            Self::NegY => [0, -1, 0],
            Self::PosY => [0, 1, 0],
            Self::NegZ => [0, 0, -1],
            Self::PosZ => [0, 0, 1],
            Self::Inside => [0, 0, 0],
        }
    }
}

/// The first solid cell along a ray: an integer record plus the distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceHit {
    /// The voxel where the ray entered the solid cell (the solid voxel itself
    /// at full resolution).
    pub voxel: VoxelPos,
    /// The level of the cell the hit was taken at: it is `4^level` voxels
    /// wide; 0 is a voxel. Equals the trace's stop level, except for a cell
    /// that is not resident, which is as wide as the unloaded region.
    pub level: u32,
    pub face: Face,
    /// The voxel's material, or the cell's representative material; air for
    /// a cell that is not resident.
    pub material: MaterialId,
    /// False when the ray reached a region whose content is not loaded
    /// (decision 0006): `voxel` and `level` then name that region's cell,
    /// which is never treated as air.
    pub resident: bool,
    /// Distance along the ray, in units of the direction's length.
    pub t: f32,
}

impl TraceHit {
    /// The voxels of the hit cell, aligned like the source's tree (relative
    /// to `source_origin`).
    pub fn cell(&self, source_origin: VoxelPos) -> VoxelBox {
        let span = 1i64 << (2 * self.level.min(MAX_LEVELS));
        let align = |v: i32, o: i32| {
            let local = i64::from(v) - i64::from(o);
            (i64::from(o) + local.div_euclid(span) * span) as i32
        };
        let min = VoxelPos::new(
            align(self.voxel.x, source_origin.x),
            align(self.voxel.y, source_origin.y),
            align(self.voxel.z, source_origin.z),
        );
        VoxelBox::from_min_size(min, [span as u32; 3])
    }
}

/// Why a trace was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceError {
    Ray(RayError),
    /// `max_t` is NaN or negative.
    BadMaxT,
    /// The source has no levels or more than [`MAX_LEVELS`].
    UnsupportedLevels(u32),
    /// The stop level is above the source's level count.
    BadStopLevel {
        stop_level: u32,
        levels: u32,
    },
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ray(e) => write!(f, "{e}"),
            Self::BadMaxT => f.write_str("max_t must not be NaN or negative"),
            Self::UnsupportedLevels(n) => {
                write!(f, "a source with {n} levels is not 1 to {MAX_LEVELS}")
            }
            Self::BadStopLevel { stop_level, levels } => {
                write!(
                    f,
                    "stop level {stop_level} is above the source's {levels} levels"
                )
            }
        }
    }
}

impl std::error::Error for TraceError {}

impl From<RayError> for TraceError {
    fn from(e: RayError) -> Self {
        Self::Ray(e)
    }
}

/// The first solid cell along `ray` in `source`, or `Ok(None)` for a miss.
/// See the module docs for the exact rules.
pub fn trace<S: VoxelSource + ?Sized>(
    source: &S,
    ray: &Ray,
    options: &TraceOptions,
) -> Result<Option<TraceHit>, TraceError> {
    ray.check()?;
    if options.max_t.is_nan() || options.max_t < 0.0 {
        return Err(TraceError::BadMaxT);
    }
    let levels = source.levels();
    if !(1..=MAX_LEVELS).contains(&levels) {
        return Err(TraceError::UnsupportedLevels(levels));
    }
    if options.stop_level > levels {
        return Err(TraceError::BadStopLevel {
            stop_level: options.stop_level,
            levels,
        });
    }
    Ok(walk::walk(source, ray, options))
}
