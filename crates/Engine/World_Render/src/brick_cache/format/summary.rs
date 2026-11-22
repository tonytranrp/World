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

use super::{PALETTE_BITS_SHIFT, PALETTE_LEN_MASK, PALETTE_PAIR_SHIFT};

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
        Content::Leaf => count_leaf(&node, tally),
        Content::Branch => {
            // A brick the source stores with its counts at hand (the world
            // tree's header, records and palettes) is counted from them,
            // without a view per leaf (decision 0024).
            if level == 1 && node.brick_counts(&mut |m, n| tally.add(m, u64::from(n))) {
                return;
            }
            for (_, child) in node.children() {
                count_voxels(child, level.saturating_sub(1), tally);
            }
        }
    }
}

/// Counts the solid voxels of a leaf per material, reading as little as the
/// source allows: a one-material palette is its mask's population, a
/// packed palette block is counted from its indices, and anything else is
/// read voxel by voxel. The world tree keeps both exact (a stored palette
/// may hold unused entries only when the leaf is packed, and those count
/// zero), so every path gives the voxel-by-voxel answer.
pub fn count_leaf<N: NodeView>(leaf: &N, tally: &mut Tally) {
    let mask = leaf.mask();
    if let Some(block) = leaf.palette_block() {
        count_palette_block(block, mask, tally);
    } else if let [m] = leaf.palette().as_slice() {
        tally.add(*m, u64::from(mask.count_ones()));
    } else {
        for slot in 0..64 {
            tally.add(leaf.voxel(slot), 1);
        }
    }
}

/// Counts a packed palette block's solid voxels per material: one index per
/// set bit of `leaf_mask`, in the layout of decisions 0005 and 0025 (header
/// `len | bits << 8` from three materials on, the palette two ids per word,
/// then the indices LSB-first).
pub fn count_palette_block(block: &[u32], leaf_mask: u64, tally: &mut Tally) {
    let first = block[0];
    let (len, bits, palette_at) = if first >> PALETTE_PAIR_SHIFT != 0 {
        (2, 1, 0)
    } else {
        (
            (first & PALETTE_LEN_MASK) as usize,
            (first >> PALETTE_BITS_SHIFT) & 0xFF,
            1,
        )
    };
    let material = |i: usize| {
        let word = block.get(palette_at + i / 2).copied().unwrap_or(0);
        MaterialId((word >> (16 * (i % 2))) as u16)
    };
    let solid = leaf_mask.count_ones();
    if bits == 0 {
        // Every index is 0.
        if len > 0 {
            tally.add(material(0), u64::from(solid));
        }
        return;
    }
    // At most 64 voxels, so a byte per palette entry holds any count.
    let mut counts = [0u8; 64];
    let indices = &block[palette_at + len.div_ceil(2)..];
    let index_mask = (1u32 << bits) - 1;
    for k in 0..solid {
        let at = k * bits;
        let index = indices
            .get((at / 32) as usize)
            .map_or(0, |w| (w >> (at % 32)) & index_mask);
        if let Some(c) = counts.get_mut(index as usize) {
            *c += 1;
        }
    }
    for (i, &count) in counts.iter().enumerate().take(len) {
        if count > 0 {
            tally.add(material(i), u64::from(count));
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

    /// Every leaf of `node`, recursively.
    fn leaves<N: NodeView>(node: N, out: &mut Vec<N>) {
        match content(&node) {
            Content::Leaf => out.push(node),
            Content::Branch => {
                for (_, child) in node.children() {
                    leaves(child, out);
                }
            }
            _ => {}
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]
        /// The fast leaf count (palette, packed indices) gives exactly the
        /// voxel-by-voxel count, including leaves overwritten until their
        /// stored palette holds unused entries.
        #[test]
        fn leaf_counts_match_reading_every_voxel(
            ops in proptest::collection::vec((0i32..24, 0i32..24, 0i32..24, 0u16..6), 1..400),
        ) {
            let mut tree = world_core::VoxelTree::new();
            for &(x, y, z, m) in &ops {
                tree.set(VoxelPos::new(x, y, z), MaterialId(m)).unwrap();
            }
            let snap = tree.snapshot();
            let mut found = Vec::new();
            leaves(snap.root(), &mut found);
            for leaf in found {
                let mut fast = Tally::default();
                count_leaf(&leaf, &mut fast);
                let mut slow = Tally::default();
                for slot in 0..64 {
                    slow.add(leaf.voxel(slot), 1);
                }
                proptest::prop_assert_eq!(fast.summary(), slow.summary());
                proptest::prop_assert_eq!(fast.counts, slow.counts);
            }
        }
    }

    #[test]
    fn spans_and_volumes() {
        assert_eq!(node_span(0), 4);
        assert_eq!(node_span(1), 16);
        assert_eq!(node_volume(9), 1u64 << 60);
    }
}
