//! Axis-aligned boxes of voxels.

use super::VoxelPos;
use crate::consts::{WORLD_END_VOXEL, WORLD_MIN_VOXEL};

/// A half-open box of voxels: every `p` with `min <= p < end` on each axis.
/// A box whose `end` is not above `min` on some axis is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoxelBox {
    min: VoxelPos,
    end: VoxelPos,
}

impl VoxelBox {
    /// Every voxel of the world.
    pub const WORLD: Self = Self::new(
        VoxelPos::new(WORLD_MIN_VOXEL, WORLD_MIN_VOXEL, WORLD_MIN_VOXEL),
        VoxelPos::new(WORLD_END_VOXEL, WORLD_END_VOXEL, WORLD_END_VOXEL),
    );

    pub const fn new(min: VoxelPos, end: VoxelPos) -> Self {
        Self { min, end }
    }

    /// The box starting at `min` with `size` voxels per axis. An end that would
    /// overflow `i32` is clamped to `i32::MAX`.
    pub const fn from_min_size(min: VoxelPos, size: [u32; 3]) -> Self {
        const fn end(start: i32, size: u32) -> i32 {
            let e = start as i64 + size as i64;
            if e > i32::MAX as i64 {
                i32::MAX
            } else {
                e as i32
            }
        }
        Self::new(
            min,
            VoxelPos::new(
                end(min.x, size[0]),
                end(min.y, size[1]),
                end(min.z, size[2]),
            ),
        )
    }

    pub const fn min(&self) -> VoxelPos {
        self.min
    }

    /// One past the highest voxel on each axis.
    pub const fn end(&self) -> VoxelPos {
        self.end
    }

    pub const fn is_empty(&self) -> bool {
        self.end.x <= self.min.x || self.end.y <= self.min.y || self.end.z <= self.min.z
    }

    /// Voxels per axis; zero on an empty axis.
    pub const fn size(&self) -> [u32; 3] {
        const fn len(a: i32, b: i32) -> u32 {
            if b > a {
                (b as i64 - a as i64) as u32
            } else {
                0
            }
        }
        [
            len(self.min.x, self.end.x),
            len(self.min.y, self.end.y),
            len(self.min.z, self.end.z),
        ]
    }

    /// Number of voxels in the box.
    pub const fn volume(&self) -> u64 {
        let [x, y, z] = self.size();
        x as u64 * y as u64 * z as u64
    }

    pub const fn contains(&self, p: VoxelPos) -> bool {
        p.x >= self.min.x
            && p.x < self.end.x
            && p.y >= self.min.y
            && p.y < self.end.y
            && p.z >= self.min.z
            && p.z < self.end.z
    }

    /// Whether every voxel of `other` is inside this box. An empty box is
    /// contained in every box.
    pub const fn contains_box(&self, other: &VoxelBox) -> bool {
        other.is_empty()
            || (other.min.x >= self.min.x
                && other.min.y >= self.min.y
                && other.min.z >= self.min.z
                && other.end.x <= self.end.x
                && other.end.y <= self.end.y
                && other.end.z <= self.end.z)
    }

    /// The voxels in both boxes (possibly empty).
    pub fn intersection(&self, other: &VoxelBox) -> VoxelBox {
        VoxelBox::new(
            VoxelPos::new(
                self.min.x.max(other.min.x),
                self.min.y.max(other.min.y),
                self.min.z.max(other.min.z),
            ),
            VoxelPos::new(
                self.end.x.min(other.end.x),
                self.end.y.min(other.end.y),
                self.end.z.min(other.end.z),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_boxes_have_no_volume() {
        let b = VoxelBox::new(VoxelPos::new(0, 0, 0), VoxelPos::new(4, 0, 4));
        assert!(b.is_empty());
        assert_eq!(b.volume(), 0);
        assert!(!b.contains(VoxelPos::new(1, 0, 1)));
    }

    #[test]
    fn world_box_spans_the_world() {
        assert_eq!(
            VoxelBox::WORLD.size(),
            [crate::consts::WORLD_SIZE_VOXELS; 3]
        );
        assert!(VoxelBox::WORLD.contains(VoxelPos::ORIGIN));
    }

    #[test]
    fn intersection_is_the_overlap() {
        let a = VoxelBox::from_min_size(VoxelPos::new(-4, -4, -4), [8, 8, 8]);
        let b = VoxelBox::from_min_size(VoxelPos::new(2, -10, 0), [10, 8, 1]);
        let i = a.intersection(&b);
        assert_eq!(
            i,
            VoxelBox::new(VoxelPos::new(2, -4, 0), VoxelPos::new(4, -2, 1))
        );
        assert!(a.contains_box(&i) && b.contains_box(&i));
    }
}
