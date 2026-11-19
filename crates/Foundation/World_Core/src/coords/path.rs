//! Paths down the world tree: which child slot a voxel falls in at each level.
//!
//! Positions are first made world-local (`pos - WORLD_MIN_VOXEL`, so every
//! axis is in `0..WORLD_SIZE_VOXELS`). At level `L` the two bits at `2L` of
//! each local axis pick the child: slot `x + 4y + 16z`. This math is a format
//! fact the GPU mirror must reproduce bit for bit (decision 0004).

use super::VoxelPos;
use crate::consts::{TREE_BRANCH_DIM, WORLD_MIN_VOXEL};

/// The world-local coordinate of a voxel, or `None` outside the world.
pub const fn world_local(pos: VoxelPos) -> Option<[u32; 3]> {
    if !pos.in_world() {
        return None;
    }
    Some([
        (pos.x - WORLD_MIN_VOXEL) as u32,
        (pos.y - WORLD_MIN_VOXEL) as u32,
        (pos.z - WORLD_MIN_VOXEL) as u32,
    ])
}

/// The voxel at a world-local coordinate. Inverse of [`world_local`] for
/// coordinates below `WORLD_SIZE_VOXELS`.
pub const fn from_world_local(local: [u32; 3]) -> VoxelPos {
    VoxelPos::new(
        local[0] as i32 + WORLD_MIN_VOXEL,
        local[1] as i32 + WORLD_MIN_VOXEL,
        local[2] as i32 + WORLD_MIN_VOXEL,
    )
}

/// The child slot (0 to 63) that a world-local coordinate falls in at `level`.
pub const fn child_slot(local: [u32; 3], level: u32) -> u32 {
    let shift = 2 * level;
    ((local[0] >> shift) & 3) | (((local[1] >> shift) & 3) << 2) | (((local[2] >> shift) & 3) << 4)
}

/// The (x, y, z) offset of a child slot inside its node, each 0 to 3.
pub const fn slot_offset(slot: u32) -> [u32; 3] {
    [slot & 3, (slot >> 2) & 3, (slot >> 4) & 3]
}

/// Voxels per axis covered by one child of a level-`level` node: `4^level`.
pub const fn child_span(level: u32) -> u32 {
    TREE_BRANCH_DIM.pow(level)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::{TREE_DEPTH, WORLD_END_VOXEL, WORLD_SIZE_VOXELS};

    #[test]
    fn local_round_trips_and_rejects_outside() {
        for p in [
            VoxelPos::ORIGIN,
            VoxelPos::new(WORLD_MIN_VOXEL, WORLD_END_VOXEL - 1, -7),
        ] {
            assert_eq!(from_world_local(world_local(p).unwrap()), p);
        }
        assert_eq!(world_local(VoxelPos::new(WORLD_END_VOXEL, 0, 0)), None);
    }

    #[test]
    fn slots_and_offsets_are_inverse() {
        for slot in 0..64 {
            let [x, y, z] = slot_offset(slot);
            assert_eq!(child_slot([x, y, z], 0), slot);
            assert_eq!(child_slot([x << 6, y << 6, z << 6], 3), slot);
        }
    }

    #[test]
    fn the_root_covers_the_whole_world() {
        assert_eq!(
            child_span(TREE_DEPTH - 1) * TREE_BRANCH_DIM,
            WORLD_SIZE_VOXELS
        );
    }
}
