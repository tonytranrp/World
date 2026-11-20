//! Voxel edits: what a change is, why it can fail, and what it touched.

use std::fmt;

use crate::coords::{BrickPos, VoxelPos};
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

/// Why an edit was refused. A refused edit or batch changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// The voxel lies outside the fixed world.
    OutOfWorld { pos: VoxelPos },
    /// A whole-node operation was given a position that is not the lowest
    /// voxel of a node at `level`.
    Misaligned { pos: VoxelPos, level: u32 },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfWorld { pos } => write!(
                f,
                "voxel ({}, {}, {}) is outside the world",
                pos.x, pos.y, pos.z
            ),
            Self::Misaligned { pos, level } => write!(
                f,
                "voxel ({}, {}, {}) is not the lowest corner of a level-{level} node",
                pos.x, pos.y, pos.z
            ),
        }
    }
}

impl std::error::Error for EditError {}

/// The bricks whose content an edit batch changed, sorted and without
/// duplicates. Empty when nothing changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirtyBricks(Vec<BrickPos>);

impl DirtyBricks {
    /// Builds the set from any list of bricks: sorts and removes duplicates.
    pub fn from_unsorted(mut bricks: Vec<BrickPos>) -> Self {
        bricks.sort_unstable();
        bricks.dedup();
        Self(bricks)
    }

    pub fn as_slice(&self) -> &[BrickPos] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn contains(&self, brick: BrickPos) -> bool {
        self.0.binary_search(&brick).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_bricks_are_sorted_and_unique() {
        let d = DirtyBricks::from_unsorted(vec![
            BrickPos::new(1, 0, 0),
            BrickPos::new(-1, 0, 0),
            BrickPos::new(1, 0, 0),
        ]);
        assert_eq!(
            d.as_slice(),
            [BrickPos::new(-1, 0, 0), BrickPos::new(1, 0, 0)]
        );
        assert!(d.contains(BrickPos::new(-1, 0, 0)));
        assert!(!d.contains(BrickPos::new(0, 0, 0)));
    }

    #[test]
    fn out_of_world_message_names_the_voxel() {
        let e = EditError::OutOfWorld {
            pos: VoxelPos::new(1, -2, 3),
        };
        assert!(e.to_string().contains("(1, -2, 3)"));
    }
}
