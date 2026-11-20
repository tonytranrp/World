//! Building whole subtrees from a description of their content (decision
//! 0006).
//!
//! A [`RegionSource`] answers two questions: is this cube all one material,
//! and what are the 64 voxels of this leaf. The builder asks the first
//! question from the top down and descends only where the answer is "mixed or
//! unknown", then writes canonical nodes bottom-up without reading anything
//! back. At the bottom it asks for a level-1 node's 64 leaves at once
//! ([`RegionSource::brick`]), so a source can share work between them.
//! Terrain generators describe heightfields this way, so solid ground and
//! open sky cost one question per node instead of one write per voxel.

use crate::consts::TREE_FANOUT;
use crate::coords::{VoxelPos, path};
use crate::material::MaterialId;

use super::mutate::Builder;
use super::node::Child;
use super::node::leaf::Record;

/// The content of a region of the world, for bulk building.
///
/// All methods must describe the same content and must be deterministic.
pub trait RegionSource {
    /// The one material of every voxel in the cube of `size` voxels per axis
    /// whose lowest voxel is `min`, or `None` when the cube is mixed or the
    /// source cannot tell cheaply. A `None` for a uniform cube only costs
    /// speed; a material for a mixed cube builds wrong content.
    fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId>;

    /// The 64 voxels of the 4x4x4 leaf whose lowest voxel is `min`, in slot
    /// order: voxel `(x, y, z)` of the leaf at index `x + 4y + 16z`.
    fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]);

    /// The 64 leaves of the level-1 node whose lowest voxel is `min` (16
    /// voxels per axis, `min` a multiple of 16 from the world's lowest
    /// voxel), each passed to `each` with its slot `x + 4y + 16z` (leaf
    /// `(x, y, z)` starts at `min + 4 (x, y, z)`), every slot once. A
    /// [`LeafFill::Uniform`] answer must be exact, like `uniform`'s.
    ///
    /// The default asks [`uniform`](Self::uniform) and then
    /// [`leaf`](Self::leaf) leaf by leaf ([`brick_by_leaves`]); a source that
    /// can share work between the leaves of a node overrides it.
    fn brick(&self, min: VoxelPos, each: &mut dyn FnMut(u32, LeafFill<'_>)) {
        brick_by_leaves(self, min, each);
    }
}

/// One leaf of a level-1 node, as [`RegionSource::brick`] gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafFill<'a> {
    /// Every voxel is this material (air included).
    Uniform(MaterialId),
    /// The 64 voxels in slot order; they may still all be one material.
    Voxels(&'a [MaterialId; 64]),
}

/// [`RegionSource::brick`] by asking `source` leaf by leaf: `uniform`, then
/// `leaf` where that is `None`.
pub fn brick_by_leaves<S: RegionSource + ?Sized>(
    source: &S,
    min: VoxelPos,
    each: &mut dyn FnMut(u32, LeafFill<'_>),
) {
    let mut voxels = [MaterialId::AIR; TREE_FANOUT as usize];
    for slot in 0..TREE_FANOUT {
        let [x, y, z] = path::slot_offset(slot).map(|v| 4 * v as i32);
        let leaf_min = VoxelPos::new(min.x + x, min.y + y, min.z + z);
        match source.uniform(leaf_min, 4) {
            Some(material) => each(slot, LeafFill::Uniform(material)),
            None => {
                source.leaf(leaf_min, &mut voxels);
                each(slot, LeafFill::Voxels(&voxels));
            }
        }
    }
}

/// Builds the child that stands where a node of `level` covers the cube at
/// world-local `origin`.
pub(crate) fn child(
    builder: &mut Builder<'_>,
    source: &impl RegionSource,
    level: u32,
    origin: [u32; 3],
) -> Child {
    let size = path::child_span(level + 1);
    if let Some(material) = source.uniform(path::from_world_local(origin), size) {
        return Child::filled(material);
    }
    if builder.enters_chunk(level) {
        builder.begin_chunk();
        let root = mixed(builder, source, level, origin);
        return builder.end_chunk_at(root, origin);
    }
    mixed(builder, source, level, origin)
}

/// Builds the node at `level` and `origin` whose content is mixed or unknown.
fn mixed(
    builder: &mut Builder<'_>,
    source: &impl RegionSource,
    level: u32,
    origin: [u32; 3],
) -> Child {
    let span = path::child_span(level);
    let child_origin = |slot: u32| {
        let offset = path::slot_offset(slot);
        [0, 1, 2].map(|a| origin[a] + offset[a] * span)
    };
    if level == 1 {
        let mut records = [Record::AIR; TREE_FANOUT as usize];
        source.brick(path::from_world_local(origin), &mut |slot, fill| {
            records[slot as usize] = match fill {
                LeafFill::Uniform(material) => Record::filled(material),
                LeafFill::Voxels(voxels) => builder.leaf(voxels),
            };
        });
        return builder.brick(&records);
    }
    let mut children = [Child::Air; TREE_FANOUT as usize];
    for (slot, slot_child) in (0..).zip(children.iter_mut()) {
        *slot_child = child(builder, source, level - 1, child_origin(slot));
    }
    builder.branch(level, &children)
}
