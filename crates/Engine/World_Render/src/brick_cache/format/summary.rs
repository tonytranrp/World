//! Representative materials: what a coarse cell is drawn with.
//!
//! The definition, exact and deterministic:
//!
//! - **Leaf (4³) and level-1 node (16³, one brick):** the most common solid
//!   material among its voxels, counted voxel by voxel. Ties go to the lower
//!   material id.
//! - **Node of level 2 and up:** the weighted mode of its non-air children's
//!   representative materials, each weighted by that child's solid voxel
//!   count (a uniform child weighs its full volume). Ties go to the lower id.
//!
//! Above level 1 this is not always the exact mode of the subtree (a material
//! that is second everywhere can be first overall), but it is computed
//! bottom-up from 64 children per node, so an edit re-evaluates one path
//! instead of the whole subtree. Air has no representative; an all-air node
//! has none and reports air.

use world_core::MaterialId;
use world_core::tree::{NodeKind, NodeView};

/// What a source child is, as the GPU format sees it. The only place that
/// matches World_Core's `NodeKind`, so a new kind is one line here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    Air,
    Uniform(MaterialId),
    /// Mixed children (level 1 and up).
    Branch,
    /// A 4³ leaf of mixed voxels.
    Leaf,
    /// Content the CPU tree has not loaded (decision 0006). Drawn as an empty
    /// not-resident node (mask 0, material air); nothing under it is uploaded.
    Unloaded,
}

/// The content of a view.
pub fn content<N: NodeView>(node: &N) -> Content {
    match node.kind() {
        NodeKind::Air => Content::Air,
        NodeKind::Uniform(m) => Content::Uniform(m),
        NodeKind::Branch => Content::Branch,
        NodeKind::Leaf => Content::Leaf,
        NodeKind::NotResident => Content::Unloaded,
    }
}

/// What a node holds, as far as coarse drawing cares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// The representative material; air when the node is all air.
    pub material: MaterialId,
    /// Solid voxels in the node.
    pub solid: u64,
}

/// Voxels per axis of a node of `level` (a level-0 node is a 4³ leaf).
pub const fn node_span(level: u32) -> u64 {
    1u64 << (2 * (level + 1))
}

/// Voxels in a node of `level`.
pub const fn node_volume(level: u32) -> u64 {
    let s = node_span(level);
    s * s * s
}

/// Weighted counts per material, kept small and sorted by id.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    counts: Vec<(MaterialId, u64)>,
}

impl Tally {
    pub fn clear(&mut self) {
        self.counts.clear();
    }

    pub fn add(&mut self, material: MaterialId, weight: u64) {
        if material.is_air() || weight == 0 {
            return;
        }
        match self.counts.binary_search_by_key(&material, |&(m, _)| m) {
            Ok(i) => self.counts[i].1 += weight,
            Err(i) => self.counts.insert(i, (material, weight)),
        }
    }

    /// The material with the largest weight, the lower id on ties, and the
    /// total weight. Air with zero weight when nothing was added.
    pub fn summary(&self) -> Summary {
        let mut best = (MaterialId::AIR, 0u64);
        let mut total = 0u64;
        // Ascending ids with a strict `>` keep the lower id on ties.
        for &(m, w) in &self.counts {
            total += w;
            if w > best.1 {
                best = (m, w);
            }
        }
        Summary {
            material: best.0,
            solid: total,
        }
    }
}

/// The summary of a level-0 or level-1 node by counting its voxels.
/// `level` is the node's level (0 for a leaf view, 1 for a brick view).
pub fn count_voxels<N: NodeView>(node: N, level: u32, tally: &mut Tally) {
    match content(&node) {
        Content::Air | Content::Unloaded => {}
        Content::Uniform(m) => tally.add(m, node_volume(level)),
        Content::Leaf => {
            for slot in 0..64 {
                tally.add(node.voxel(slot), 1);
            }
        }
        Content::Branch => {
            for (_, child) in node.children() {
                count_voxels(child, level.saturating_sub(1), tally);
            }
        }
    }
}

/// The exact summary of a brick (a level-1 node) or a leaf.
pub fn exact_summary<N: NodeView>(node: N, level: u32) -> Summary {
    let mut tally = Tally::default();
    count_voxels(node, level, &mut tally);
    tally.summary()
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::{DenseGrid, VoxelPos, VoxelSource};

    #[test]
    fn ties_go_to_the_lower_id() {
        let mut t = Tally::default();
        t.add(MaterialId(9), 4);
        t.add(MaterialId(3), 4);
        t.add(MaterialId(5), 2);
        assert_eq!(
            t.summary(),
            Summary {
                material: MaterialId(3),
                solid: 10
            }
        );
    }

    #[test]
    fn air_is_never_representative() {
        let mut t = Tally::default();
        t.add(MaterialId::AIR, 100);
        assert_eq!(t.summary(), Summary::default());
    }

    #[test]
    fn a_brick_counts_every_voxel() {
        // A 2-level grid is one brick (16³).
        let mut g = DenseGrid::new(2, VoxelPos::new(0, 0, 0));
        for x in 0..3 {
            g.set(VoxelPos::new(x, 0, 0), MaterialId(2));
        }
        for x in 0..2 {
            g.set(VoxelPos::new(x, 5, 9), MaterialId(1));
        }
        let s = exact_summary(g.root(), 1);
        assert_eq!(
            s,
            Summary {
                material: MaterialId(2),
                solid: 5
            }
        );
    }

    #[test]
    fn spans_and_volumes() {
        assert_eq!(node_span(0), 4);
        assert_eq!(node_span(1), 16);
        assert_eq!(node_volume(9), 1u64 << 60);
    }
}
