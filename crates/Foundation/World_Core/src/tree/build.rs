//! Building whole subtrees from a description of their content (decision
//! 0006).
//!
//! A [`RegionSource`] answers two questions: is this cube all one material,
//! and what are the 64 voxels of this leaf. The builder asks the first
//! question from the top down and descends only where the answer is "mixed or
//! unknown", then writes canonical nodes bottom-up without reading anything
//! back. Terrain generators describe heightfields this way, so solid ground
//! and open sky cost one question per node instead of one write per voxel.

use crate::consts::TREE_FANOUT;
use crate::coords::{VoxelPos, path};
use crate::material::MaterialId;

use super::mutate::Builder;
use super::node::Child;
use super::node::leaf::{Record, Voxels};

/// The content of a region of the world, for bulk building.
///
/// Both methods must describe the same content and must be deterministic.
pub trait RegionSource {
    /// The one material of every voxel in the cube of `size` voxels per axis
    /// whose lowest voxel is `min`, or `None` when the cube is mixed or the
    /// source cannot tell cheaply. A `None` for a uniform cube only costs
    /// speed; a material for a mixed cube builds wrong content.
    fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId>;

    /// The 64 voxels of the 4x4x4 leaf whose lowest voxel is `min`, in slot
    /// order: voxel `(x, y, z)` of the leaf at index `x + 4y + 16z`.
    fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]);
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
        return builder.end_chunk(root);
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
        let mut voxels: Voxels = [MaterialId::AIR; TREE_FANOUT as usize];
        for (slot, record) in (0..).zip(records.iter_mut()) {
            let min = path::from_world_local(child_origin(slot));
            *record = match source.uniform(min, span) {
                Some(material) => Record::filled(material),
                None => {
                    source.leaf(min, &mut voxels);
                    builder.leaf(&voxels)
                }
            };
        }
        return builder.brick(&records);
    }
    let mut children = [Child::Air; TREE_FANOUT as usize];
    for (slot, slot_child) in (0..).zip(children.iter_mut()) {
        *slot_child = child(builder, source, level - 1, child_origin(slot));
    }
    builder.branch(level, &children)
}
