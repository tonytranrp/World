//! The single home of the workspace's shared constants.
//!
//! Anything the GPU must agree on is defined here so Xtask can generate the
//! matching WESL module. Never redefine these values elsewhere.

/// Render voxels per metre: the world is stored at 1/64 m (1.5625 cm).
pub const VOXELS_PER_METER: u32 = 64;

/// Edge length of one render voxel, in metres.
pub const VOXEL_SIZE_M: f32 = 1.0 / VOXELS_PER_METER as f32;

/// Collision voxels per metre: physics collides on a coarser 1/16 m grid.
pub const COLLISION_VOXELS_PER_METER: u32 = 16;

/// Edge length of one collision voxel, in metres.
pub const COLLISION_VOXEL_SIZE_M: f32 = 1.0 / COLLISION_VOXELS_PER_METER as f32;

/// Render voxels along one edge of a collision voxel.
pub const COLLISION_SCALE: u32 = VOXELS_PER_METER / COLLISION_VOXELS_PER_METER;

/// Children per axis of a 64-tree node (4x4x4 children).
pub const TREE_BRANCH_DIM: u32 = 4;

/// Children per 64-tree node; also the width of the child occupancy mask in bits.
pub const TREE_FANOUT: u32 = TREE_BRANCH_DIM * TREE_BRANCH_DIM * TREE_BRANCH_DIM;

/// Node levels in the world tree. A level-`L` node's children each cover
/// `4^L` voxels per axis: level 0 holds single voxels (a leaf), and the root is
/// level `TREE_DEPTH - 1`.
pub const TREE_DEPTH: u32 = 10;

/// Voxels along one edge of the world: `4^TREE_DEPTH`, 16.384 km.
pub const WORLD_SIZE_VOXELS: u32 = TREE_BRANCH_DIM.pow(TREE_DEPTH);

/// The lowest voxel coordinate inside the world, on every axis.
pub const WORLD_MIN_VOXEL: i32 = -(WORLD_SIZE_VOXELS as i32 / 2);

/// One past the highest voxel coordinate inside the world, on every axis.
pub const WORLD_END_VOXEL: i32 = WORLD_SIZE_VOXELS as i32 / 2;

/// Voxels along one edge of a brick: the region of one level-1 node, and the
/// unit of change reports and GPU streaming.
pub const BRICK_DIM: u32 = TREE_BRANCH_DIM * TREE_BRANCH_DIM;

const _: () = assert!(VOXELS_PER_METER.is_multiple_of(COLLISION_VOXELS_PER_METER));
const _: () = assert!(TREE_FANOUT == 64);
const _: () = assert!(WORLD_SIZE_VOXELS == 1 << 20);
const _: () = assert!(WORLD_END_VOXEL - WORLD_MIN_VOXEL == WORLD_SIZE_VOXELS as i32);
const _: () = assert!(BRICK_DIM == 16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voxel_size_is_one_sixty_fourth_of_a_metre() {
        assert_eq!(VOXEL_SIZE_M, 0.015625);
    }

    #[test]
    fn collision_grid_is_four_render_voxels_wide() {
        assert_eq!(COLLISION_SCALE, 4);
        assert_eq!(
            COLLISION_VOXEL_SIZE_M,
            VOXEL_SIZE_M * COLLISION_SCALE as f32
        );
    }

    #[test]
    fn world_is_about_sixteen_kilometres_wide() {
        let metres = WORLD_SIZE_VOXELS as f32 * VOXEL_SIZE_M;
        assert_eq!(metres, 16_384.0);
    }

    #[test]
    fn tree_fanout_fills_a_64_bit_mask() {
        assert_eq!(TREE_FANOUT, u64::BITS);
    }
}
