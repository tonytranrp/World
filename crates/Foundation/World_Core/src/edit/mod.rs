//! Voxel edits.

use crate::coords::VoxelPos;
use crate::material::MaterialId;

/// Set one voxel to a material. Setting air removes the voxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Edit {
    pub pos: VoxelPos,
    pub material: MaterialId,
}

impl Edit {
    pub const fn new(pos: VoxelPos, material: MaterialId) -> Self {
        Self { pos, material }
    }
}
