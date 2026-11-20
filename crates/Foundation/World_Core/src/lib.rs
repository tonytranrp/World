//! World_Core: voxel types, the sparse 64-tree, materials, edits and serialization.
//!
//! This crate has no Bevy or GPU dependency. See the folder READMEs for the
//! module layout and `docs/decisions/0004-voxel-core.md` for the tree design;
//! the shared constants live in [`consts`].

pub mod config;
pub mod consts;
pub mod coords;
pub mod edit;
pub mod material;
pub mod serialize;
pub mod tree;

pub use config::ConfigAsset;
pub use coords::{BrickPos, VoxelBox, VoxelPos};
pub use edit::{DirtyBricks, Edit, EditError};
pub use material::{MaterialDef, MaterialId, MaterialTable};
pub use tree::{DenseGrid, TreeCursor, TreeRead, TreeSnapshot, VoxelSource, VoxelTree};
