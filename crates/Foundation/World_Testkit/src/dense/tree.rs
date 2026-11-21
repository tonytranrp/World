//! The production 64-tree and dense object grids seen through the oracle's
//! eyes: adapters so they can be compared with [`DenseWorld`](super::DenseWorld),
//! a comparison for any voxel source (live tree, snapshot or dense grid), and
//! a walk that checks the canonical form of decision 0004 node by node.

use std::collections::{BTreeMap, BTreeSet};

use world_core::tree::{CHUNK_LEVEL, NodeKind, NodeView, TreeRead, TreeStats, VoxelSource};
use world_core::{DenseGrid, MaterialId, VoxelBox, VoxelPos, VoxelTree};

use super::diverge::{Divergence, first_difference};
use super::world::{Bounds, VoxelModel};

/// The tree follows the dense model's policy: a write the store refuses is a
/// bug in the test, so it panics with the tree's own error message.
impl VoxelModel for VoxelTree {
    fn get(&self, pos: VoxelPos) -> MaterialId {
        TreeRead::get(self, pos)
    }

    fn set(&mut self, pos: VoxelPos, material: MaterialId) {
        if let Err(e) = VoxelTree::set(self, pos, material) {
            panic!("{e}");
        }
    }

    fn fill_box(&mut self, region: VoxelBox, material: MaterialId) {
        if let Err(e) = VoxelTree::fill_box(self, region, material) {
            panic!("{e}");
        }
    }
}

/// A dense object grid is a model too. Reads outside it return air and
/// writes outside it panic, the dense model's own policy.
impl VoxelModel for DenseGrid {
    fn get(&self, pos: VoxelPos) -> MaterialId {
        DenseGrid::get(self, pos)
    }

    fn set(&mut self, pos: VoxelPos, material: MaterialId) {
        DenseGrid::set(self, pos, material);
    }
}

/// Compares any voxel source, such as a snapshot, with a model over
/// `bounds`. `left` in the divergence is the source.
pub fn assert_tree_agrees(
    tree: &impl VoxelSource,
    model: &impl VoxelModel,
    bounds: Bounds,
) -> Result<(), Divergence> {
    first_difference(bounds, |p| tree.get(p), |p| model.get(p))
}

/// Node and word totals of a tree, as `world_core::tree::node` lays it out:
/// a branch of level 2 or higher holds one word per non-air child, a level-1
/// branch one three-word record per non-air leaf, and a leaf with two or more
/// materials a palette block (a header, two ids per word, then its packed
/// indices in whole words). Leaves count those mixed children; they are not
/// nodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeCounts {
    pub branches: usize,
    pub leaves: usize,
    pub words: usize,
}

impl From<TreeStats> for NodeCounts {
    /// The live part of the store's own figures.
    fn from(stats: TreeStats) -> Self {
        Self {
            branches: stats.branches,
            leaves: stats.leaves,
            words: stats.words,
        }
    }
}

/// Walks every node of any voxel source and checks the canonical form: kinds
/// at the right levels, no empty or uniform-equivalent node, children listed
/// exactly as the mask says, leaf palettes sorted, unique, free of air and
/// holding exactly the materials present. Returns what the walk counted, as
/// the world tree would store that content, or the first problem with the
/// slot path that leads to it.
pub fn check_canonical(source: &impl VoxelSource) -> Result<NodeCounts, String> {
    let mut walk = Walk::default();
    walk.child(source.root(), source.levels() - 1)?;
    Ok(walk.counts)
}

#[derive(Default)]
struct Walk {
    path: Vec<u32>,
    counts: NodeCounts,
}

impl Walk {
    fn fail(&self, what: &str) -> String {
        format!("at slot path {:?}: {what}", self.path)
    }

    /// One child value. `level` is the level a node in its place would have.
    fn child(&mut self, node: impl NodeView, level: u32) -> Result<(), String> {
        match node.kind() {
            NodeKind::Air if node.mask() != 0 => Err(self.fail("an air child with a mask")),
            NodeKind::Air => Ok(()),
            NodeKind::Uniform(m) if m.is_air() => Err(self.fail("a uniform child of air")),
            NodeKind::Uniform(_) if node.mask() != u64::MAX => {
                Err(self.fail("a uniform child without a full mask"))
            }
            NodeKind::Uniform(_) => Ok(()),
            NodeKind::Branch if level == 0 => Err(self.fail("a branch at level 0")),
            NodeKind::Branch => self.branch(node, level),
            NodeKind::Leaf if level != 0 => Err(self.fail("a leaf above level 0")),
            NodeKind::Leaf => self.leaf(node),
            NodeKind::NotResident if level < CHUNK_LEVEL => {
                Err(self.fail("a not-resident child below the chunk level"))
            }
            NodeKind::NotResident => Ok(()),
        }
    }

    fn branch<N: NodeView>(&mut self, b: N, level: u32) -> Result<(), String> {
        let mask = b.mask();
        if mask == 0 {
            return Err(self.fail("a branch with no children"));
        }
        let listed: Vec<(u32, N)> = b.children().collect();
        if !listed.iter().map(|&(s, _)| s).eq(slots(mask)) {
            return Err(self.fail("children() does not list exactly the mask's slots in order"));
        }
        for slot in 0..64 {
            let set = mask >> slot & 1 == 1;
            let child = summary(b.child(slot));
            let expected = listed
                .iter()
                .find(|&&(s, _)| s == slot)
                .map_or(AIR_SUMMARY, |&(_, n)| summary(n));
            if (child == AIR_SUMMARY) == set || child != expected {
                return Err(self.fail(&format!("child({slot}) disagrees with the mask")));
            }
        }
        if mask == u64::MAX {
            let first = summary(listed[0].1);
            if matches!(
                listed[0].1.kind(),
                NodeKind::Uniform(_) | NodeKind::NotResident
            ) && listed.iter().all(|&(_, n)| summary(n) == first)
            {
                return Err(self.fail("a branch whose children are all one material"));
            }
        }
        self.counts.branches += 1;
        if level == 1 {
            // A header, one word per full leaf of another material and a
            // three-word record per partial leaf; the full leaves of the
            // default material (the most common material among full leaves,
            // ties to the lower id) have no words. The header is three
            // words, five with the other full leaves' mask (decision 0021).
            let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
            for (_, n) in &listed {
                if let NodeKind::Uniform(m) = n.kind() {
                    *counts.entry(m.0).or_default() += 1;
                }
            }
            let full: usize = counts.values().sum();
            let defaults = counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
                .map_or(0, |(_, &n)| n);
            let others = full - defaults;
            let header = if others == 0 { 3 } else { 5 };
            self.counts.words += header + others + 3 * (listed.len() - full);
        } else {
            self.counts.words += listed.len();
        }
        for (slot, node) in listed {
            if node.kind() == NodeKind::Air {
                return Err(self.fail("children() lists an air child"));
            }
            self.path.push(slot);
            self.child(node, level - 1)?;
            self.path.pop();
        }
        Ok(())
    }

    fn leaf(&mut self, l: impl NodeView) -> Result<(), String> {
        let mask = l.mask();
        if mask == 0 {
            return Err(self.fail("a leaf with no solid voxels"));
        }
        let palette = l.palette();
        let palette = palette.as_slice();
        if palette.is_empty() {
            return Err(self.fail("an empty palette"));
        }
        if palette.iter().any(|m| m.is_air()) || !palette.windows(2).all(|w| w[0] < w[1]) {
            return Err(self.fail(&format!(
                "a palette that is not sorted, unique and solid: {palette:?}"
            )));
        }
        let mut present = BTreeSet::new();
        for slot in 0..64 {
            let v = l.voxel(slot);
            if v.is_air() == (mask >> slot & 1 == 1) {
                return Err(self.fail(&format!("voxel {slot} disagrees with the mask")));
            }
            if !v.is_air() {
                present.insert(v);
            }
        }
        if !present.iter().eq(palette.iter()) {
            return Err(self.fail(&format!(
                "palette {palette:?} is not the materials present {present:?}"
            )));
        }
        if mask == u64::MAX && palette.len() == 1 {
            return Err(self.fail("a leaf that is all one material"));
        }
        let solids = mask.count_ones() as usize;
        self.counts.leaves += 1;
        // One material fits in the record; more need a palette block: a
        // header, two ids per word, then the packed indices.
        if palette.len() > 1 {
            self.counts.words +=
                1 + palette.len().div_ceil(2) + (solids * index_bits(palette.len())).div_ceil(32);
        }
        Ok(())
    }
}

/// The set bits of a mask, ascending.
fn slots(mask: u64) -> impl Iterator<Item = u32> {
    (0..64).filter(move |s| mask >> s & 1 == 1)
}

/// The summary of an air child.
const AIR_SUMMARY: (u8, u64) = (0, 0);

/// What a child is, without its identity: the kind plus its id or mask.
fn summary(node: impl NodeView) -> (u8, u64) {
    match node.kind() {
        NodeKind::Air => AIR_SUMMARY,
        NodeKind::Uniform(m) => (1, m.0.into()),
        NodeKind::Branch => (2, node.mask()),
        NodeKind::Leaf => (3, node.mask()),
        NodeKind::NotResident => (4, 0),
    }
}

/// The smallest of 0, 1, 2, 4 or 8 bits that can index `len` palette entries.
fn index_bits(len: usize) -> usize {
    match len {
        0 | 1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    }
}

#[cfg(test)]
mod tests {
    use world_core::tree::NodeRef;

    use super::*;

    #[test]
    fn index_width_is_the_smallest_power_of_two_that_fits() {
        let widths: Vec<usize> = [1, 2, 3, 4, 5, 16, 17, 64].map(index_bits).to_vec();
        assert_eq!(widths, [0, 1, 2, 2, 4, 4, 8, 8]);
    }

    #[test]
    fn slots_lists_set_bits_ascending() {
        assert!(slots(0b1010_0001).eq([0, 5, 7]));
        assert_eq!(slots(u64::MAX).count(), 64);
    }

    /// A tree that is one child value, which needs no store to build.
    struct Whole(NodeRef<'static>);

    impl TreeRead for Whole {
        fn root(&self) -> NodeRef<'_> {
            self.0
        }
    }

    #[test]
    fn air_and_uniform_roots_are_canonical_and_hold_no_nodes() {
        for root in [NodeRef::Air, NodeRef::Uniform(MaterialId(9))] {
            assert_eq!(check_canonical(&Whole(root)), Ok(NodeCounts::default()));
        }
    }

    #[test]
    fn uniform_air_is_rejected() {
        let err = check_canonical(&Whole(NodeRef::Uniform(MaterialId::AIR))).unwrap_err();
        assert!(err.contains("uniform child of air"), "{err}");
    }

    #[test]
    fn counts_convert_from_live_stats() {
        let stats = TreeStats {
            branches: 3,
            leaves: 2,
            words: 9,
            free_nodes: 4,
            free_words: 5,
            bytes: 6,
            chunks: 1,
        };
        assert_eq!(
            NodeCounts::from(stats),
            NodeCounts {
                branches: 3,
                leaves: 2,
                words: 9
            }
        );
    }
}
