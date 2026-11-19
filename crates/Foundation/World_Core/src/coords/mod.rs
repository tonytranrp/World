//! Integer voxel coordinates.

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
    fn origin_is_all_zero() {
        assert_eq!(VoxelPos::ORIGIN.to_array(), [0, 0, 0]);
    }
}
