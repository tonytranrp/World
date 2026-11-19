//! World_Core: voxel types, the sparse 64-tree, materials, edits and serialization.
//!
//! This crate has no Bevy or GPU dependency. See the folder READMEs for the
//! module layout; the shared constants live in [`consts`].

pub mod config;
pub mod consts;
pub mod coords;
pub mod edit;
pub mod material;

pub use config::ConfigAsset;
pub use coords::VoxelPos;
pub use edit::Edit;
pub use material::{MaterialDef, MaterialId, MaterialTable};
