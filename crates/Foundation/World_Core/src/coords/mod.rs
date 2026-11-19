//! Integer coordinates: voxels, bricks, boxes and paths down the world tree.

mod brick;
pub mod path;
mod region;

pub use brick::BrickPos;
pub use region::VoxelBox;

use crate::consts::{WORLD_END_VOXEL, WORLD_MIN_VOXEL};

/// The integer position of one voxel. The voxel at `(x, y, z)` occupies the
/// half-open cube `[x, x + 1) x [y, y + 1) x [z, z + 1)` in voxel units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VoxelPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl VoxelPos {
    pub const ORIGIN: Self = Self::new(0, 0, 0);

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub const fn to_array(self) -> [i32; 3] {
        [self.x, self.y, self.z]
    }

    /// Whether the voxel lies inside the fixed world.
    pub const fn in_world(self) -> bool {
        const fn axis(v: i32) -> bool {
            v >= WORLD_MIN_VOXEL && v < WORLD_END_VOXEL
        }
        axis(self.x) && axis(self.y) && axis(self.z)
    }

    /// The brick that contains this voxel.
    pub const fn brick(self) -> BrickPos {
        BrickPos::containing(self)
    }
}

impl From<[i32; 3]> for VoxelPos {
    fn from([x, y, z]: [i32; 3]) -> Self {
        Self::new(x, y, z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_round_trip() {
        let p = VoxelPos::new(-3, 7, 12);
        assert_eq!(VoxelPos::from(p.to_array()), p);
    }

    #[test]
    fn world_edges_are_half_open() {
        assert!(VoxelPos::new(WORLD_MIN_VOXEL, 0, 0).in_world());
        assert!(VoxelPos::new(0, WORLD_END_VOXEL - 1, 0).in_world());
        assert!(!VoxelPos::new(0, 0, WORLD_END_VOXEL).in_world());
        assert!(!VoxelPos::new(WORLD_MIN_VOXEL - 1, 0, 0).in_world());
    }

    #[test]
    fn origin_is_all_zero() {
        assert_eq!(VoxelPos::ORIGIN.to_array(), [0, 0, 0]);
    }
}
