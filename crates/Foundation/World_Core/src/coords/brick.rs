//! Brick coordinates: 16x16x16-voxel regions, one per level-1 tree node.

use super::{VoxelBox, VoxelPos};
use crate::consts::BRICK_DIM;

/// The integer position of one brick. Brick `(x, y, z)` holds the voxels from
/// `16 * (x, y, z)` up to, but not including, `16 * (x + 1, y + 1, z + 1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BrickPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BrickPos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The brick containing `pos`, rounding toward negative infinity.
    pub const fn containing(pos: VoxelPos) -> Self {
        let d = BRICK_DIM as i32;
        Self::new(
            pos.x.div_euclid(d),
            pos.y.div_euclid(d),
            pos.z.div_euclid(d),
        )
    }

    /// The lowest voxel of the brick.
    pub const fn min_voxel(self) -> VoxelPos {
        let d = BRICK_DIM as i32;
        VoxelPos::new(self.x * d, self.y * d, self.z * d)
    }

    /// Every voxel of the brick.
    pub const fn bounds(self) -> VoxelBox {
        VoxelBox::from_min_size(self.min_voxel(), [BRICK_DIM; 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_voxels_round_down() {
        assert_eq!(VoxelPos::new(-1, 0, 15).brick(), BrickPos::new(-1, 0, 0));
        assert_eq!(
            VoxelPos::new(-16, 16, -17).brick(),
            BrickPos::new(-1, 1, -2)
        );
    }

    #[test]
    fn bounds_contain_exactly_the_brick() {
        let b = BrickPos::new(-2, 0, 3);
        let bounds = b.bounds();
        assert_eq!(bounds.min(), VoxelPos::new(-32, 0, 48));
        assert_eq!(bounds.volume(), 16 * 16 * 16);
        assert!(bounds.contains(VoxelPos::new(-17, 15, 63)));
        assert!(!bounds.contains(VoxelPos::new(-16, 0, 48)));
    }
}
