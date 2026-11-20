//! The representative material of a coarse cell.

use std::collections::BTreeMap;

use world_core::MaterialId;
use world_core::tree::{NodeKind, NodeView};

/// The most common solid material among the voxels under `node`, a node of
/// tree level `level` (it covers `4^(level + 1)` voxels per axis); ties go to
/// the lowest id, and an all-air node gives air.
///
/// Computed exactly from the read interface: an air child counts nothing, a
/// uniform child of a level-`L` node counts all `64^L` of its voxels for its
/// material, a leaf counts each of its 64 voxels, and branches recurse. The
/// counts live in an ordered map, so the result never depends on hashing.
/// Slow on large mixed subtrees, which is fine for a reference.
pub fn representative<N: NodeView>(node: N, level: u32) -> MaterialId {
    let mut counts = BTreeMap::new();
    count(node, level, &mut counts);
    let mut best = (MaterialId::AIR, 0u128);
    for (&material, &n) in &counts {
        // Ascending ids: only a strictly larger count replaces the best.
        if n > best.1 {
            best = (material, n);
        }
    }
    best.0
}

/// Adds the solid voxels under `node` (tree level `level`) to `counts`.
fn count<N: NodeView>(node: N, level: u32, counts: &mut BTreeMap<MaterialId, u128>) {
    // TODO(NotResident, decision 0006): a not-resident child has no voxels
    // to count; add an arm that skips it (or carries its stored material)
    // when the variant lands. Kept exhaustive so the compiler asks.
    match node.kind() {
        NodeKind::Air => {}
        NodeKind::Uniform(m) => {
            // 64^(level + 1) voxels; level is below 15, so this fits in u128.
            *counts.entry(m).or_default() += 1u128 << (6 * (level + 1));
        }
        NodeKind::Leaf => {
            for slot in 0..64 {
                let m = node.voxel(slot);
                if !m.is_air() {
                    *counts.entry(m).or_default() += 1;
                }
            }
        }
        NodeKind::Branch => {
            for (_, child) in node.children() {
                count(child, level.saturating_sub(1), counts);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::{DenseGrid, VoxelPos, VoxelSource};

    #[test]
    fn counts_voxels_and_breaks_ties_by_lowest_id() {
        let mut g = DenseGrid::new(2, VoxelPos::ORIGIN);
        // A full leaf of 3 (64 voxels) against 70 scattered voxels of 5.
        for i in 0..64 {
            g.set(VoxelPos::new(i % 4, i / 4 % 4, i / 16), MaterialId(3));
        }
        for i in 0..70 {
            g.set(VoxelPos::new(4 + i % 12, i / 12, 8), MaterialId(5));
        }
        assert_eq!(representative(g.root(), 1), MaterialId(5));
        // Remove six: 64 each, the lower id wins.
        for i in 64..70 {
            g.set(VoxelPos::new(4 + i % 12, i / 12, 8), MaterialId::AIR);
        }
        assert_eq!(representative(g.root(), 1), MaterialId(3));
        assert_eq!(
            representative(DenseGrid::new(1, VoxelPos::ORIGIN).root(), 0),
            MaterialId::AIR
        );
    }
}
