//! Naming nodes of the mirrored tree.

use world_core::coords::path::slot_offset;
use world_core::tree::{NodeView, VoxelSource};
use world_core::{BrickPos, VoxelPos};

/// The shape of a source tree: `levels` node levels, the root at level
/// `levels - 1`, covering `4^levels` voxels per axis from `origin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub levels: u32,
    pub origin: VoxelPos,
}

impl Shape {
    pub fn of(source: &impl VoxelSource) -> Self {
        Self {
            levels: source.levels(),
            origin: source.origin(),
        }
    }

    pub const fn root_level(&self) -> u32 {
        self.levels - 1
    }

    /// Voxels per axis.
    pub const fn size(&self) -> u64 {
        1u64 << (2 * self.levels)
    }

    /// The coordinate of `pos` relative to the origin, or `None` outside.
    pub fn local(&self, pos: VoxelPos) -> Option<[u32; 3]> {
        let size = self.size() as i64;
        let p = pos.to_array();
        let o = self.origin.to_array();
        let mut out = [0u32; 3];
        for a in 0..3 {
            let d = i64::from(p[a]) - i64::from(o[a]);
            if !(0..size).contains(&d) {
                return None;
            }
            out[a] = d as u32;
        }
        Some(out)
    }

    /// The level-1 key of a brick, or `None` outside the tree. The origin of
    /// every `VoxelSource` is a multiple of 16 for trees of 2 or more levels
    /// placed on the brick grid; others are rejected.
    pub fn brick_key(&self, brick: BrickPos) -> Option<NodeKey> {
        let local = self.local(brick.min_voxel())?;
        if local.iter().any(|c| c % 16 != 0) || self.levels < 2 {
            return None;
        }
        Some(NodeKey::containing(local, 1))
    }

    /// The root's key.
    pub const fn root(&self) -> NodeKey {
        NodeKey {
            level: self.root_level() as u8,
            x: 0,
            y: 0,
            z: 0,
        }
    }
}

/// A node of level 1 or higher: its level and its position in units of its
/// own size (`4^(level + 1)` voxels per axis) from the source origin.
/// Ordered by level first, so sorting processes nodes bottom-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKey {
    pub level: u8,
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl NodeKey {
    /// The node of `level` containing local coordinate `local`.
    pub const fn containing(local: [u32; 3], level: u32) -> Self {
        let shift = 2 * (level + 1);
        Self {
            level: level as u8,
            x: local[0] >> shift,
            y: local[1] >> shift,
            z: local[2] >> shift,
        }
    }

    pub const fn level(&self) -> u32 {
        self.level as u32
    }

    /// The node's lowest local voxel coordinate.
    pub const fn min_local(&self) -> [u32; 3] {
        let shift = 2 * (self.level as u32 + 1);
        [self.x << shift, self.y << shift, self.z << shift]
    }

    /// Voxels per axis.
    pub const fn span(&self) -> u64 {
        1u64 << (2 * (self.level as u32 + 1))
    }

    /// Child `slot` (0 to 63), one level down. Only for level 2 and up.
    pub const fn child(&self, slot: u32) -> Self {
        let o = slot_offset(slot);
        Self {
            level: self.level - 1,
            x: self.x * 4 + o[0],
            y: self.y * 4 + o[1],
            z: self.z * 4 + o[2],
        }
    }

    pub const fn parent(&self) -> Self {
        Self {
            level: self.level + 1,
            x: self.x >> 2,
            y: self.y >> 2,
            z: self.z >> 2,
        }
    }

    /// This node's slot in its parent.
    pub const fn slot_in_parent(&self) -> u32 {
        (self.x & 3) | (self.y & 3) << 2 | (self.z & 3) << 4
    }

    /// The node of `level` (at or above this one) containing this node.
    pub const fn ancestor(&self, level: u32) -> Self {
        let up = 2 * (level - self.level as u32);
        Self {
            level: level as u8,
            x: self.x >> up,
            y: self.y >> up,
            z: self.z >> up,
        }
    }

    /// The brick's world position, for a level-1 key of a tree at `origin`.
    pub fn brick_pos(&self, origin: VoxelPos) -> BrickPos {
        BrickPos::containing(VoxelPos::new(
            origin.x + (self.x * 16) as i32,
            origin.y + (self.y * 16) as i32,
            origin.z + (self.z * 16) as i32,
        ))
    }
}

/// The view of the node `key` in `source`, found by descending from the root.
/// Air and uniform ancestors return themselves, as views do.
pub fn view_at<S: VoxelSource>(source: &S, key: NodeKey) -> S::Node<'_> {
    let mut node = source.root();
    let local = key.min_local();
    let mut level = source.levels() - 1;
    while level > key.level() {
        node = node.child(world_core::coords::path::child_slot(local, level));
        level -= 1;
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_and_parent_are_inverse() {
        let k = NodeKey {
            level: 3,
            x: 5,
            y: 9,
            z: 2,
        };
        for slot in [0, 1, 17, 63] {
            let c = k.child(slot);
            assert_eq!(c.parent(), k);
            assert_eq!(c.slot_in_parent(), slot);
        }
        assert_eq!(k.child(63).ancestor(3), k);
    }

    #[test]
    fn containing_matches_min_local() {
        let local = [1000, 70000, 5];
        let k = NodeKey::containing(local, 2);
        let m = k.min_local();
        for a in 0..3 {
            assert!(m[a] <= local[a] && u64::from(local[a]) < u64::from(m[a]) + k.span());
        }
    }

    #[test]
    fn brick_keys_round_trip_through_world_positions() {
        let shape = Shape {
            levels: 10,
            origin: VoxelPos::new(-(1 << 19), -(1 << 19), -(1 << 19)),
        };
        let brick = BrickPos::new(-3, 7, 100);
        let key = shape.brick_key(brick).unwrap();
        assert_eq!(key.level, 1);
        assert_eq!(key.brick_pos(shape.origin), brick);
    }
}
