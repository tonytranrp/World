//! The dense reference voxel model: a plain 3D array that is obviously correct.
//!
//! It is the oracle the sparse 64-tree is compared against under random edits,
//! and the stand-in CPU tracer until the real reference renderer exists. The
//! tree itself is adapted to [`VoxelModel`] here, next to a walk that checks
//! its canonical form, so every crate judges it the same way.

mod diverge;
mod ray;
mod tree;
mod world;

pub use diverge::{Divergence, assert_models_agree, compare_after_edits, compare_after_ops};
pub use ray::brute_force_cast;
pub use tree::{NodeCounts, assert_tree_agrees, check_canonical};
pub use world::{Bounds, DenseWorld, EditOp, VoxelModel};
