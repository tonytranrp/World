//! The dense reference voxel model: a plain 3D array that is obviously correct.
//!
//! It is the oracle the sparse 64-tree is compared against under random edits,
//! and the stand-in CPU tracer until the real reference renderer exists.

mod diverge;
mod ray;
mod world;

pub use diverge::{Divergence, assert_models_agree, compare_after_edits};
pub use ray::brute_force_cast;
pub use world::{Bounds, DenseWorld, VoxelModel};
