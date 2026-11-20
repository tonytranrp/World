//! The only write path into the tree (decision 0004).
//!
//! A write recurses from the root down the children it affects and rebuilds
//! each changed node bottom-up. Where a write lands inside an air or uniform
//! child, that child is split into a node; every rebuilt node or leaf is
//! repacked canonically, and one whose content turns out to be all one
//! material is released and becomes a filled child. Collapse is decided
//! locally: children are canonical by induction, so a node is uniform exactly
//! when its 64 child words (level 2 and up) or 64 leaf records (level 1) are
//! all the same uniform material. Released nodes, palette blocks and their
//! subtrees go back to the allocator.
//!
//! Each step returns `None` when its child did not change, so a write that
//! changes nothing allocates, copies and reports nothing. A node rebuilt in
//! place keeps its index, so its parent's word, and every page above it, is
//! left alone. Voxel writes come as a batch sorted in tree order: each node on
//! the batch's paths is visited and rebuilt once, however many of the edits
//! fall under it. A box fill visits only partially covered children and
//! replaces covered ones whole.

mod branch;
mod brick;
mod fill;
mod voxel;

pub(crate) use fill::LocalBox;
pub(crate) use voxel::LocalEdit;

use super::ROOT_LEVEL;
use super::alloc::Store;
use super::node::{Child, Node, Packed};
use crate::consts::TREE_DEPTH;
use crate::material::MaterialId;

/// Sets voxels. `edits` must be sorted by `node::path_key` with at most one
/// edit per voxel. Calls `changed` with every edit that changed its voxel and
/// the material the voxel held before.
pub(crate) fn set_voxels(
    store: &mut Store,
    root: &mut Child,
    edits: &[LocalEdit],
    changed: &mut impl FnMut(LocalEdit, MaterialId),
) {
    debug_assert!(
        edits.windows(2).all(|w| {
            let key = |e: &LocalEdit| super::node::path_key(e.local, TREE_DEPTH);
            key(&w[0]) < key(&w[1])
        }),
        "voxel edits must be in tree order, one per voxel"
    );
    let new = Writer { store }.set_voxels(*root, ROOT_LEVEL, edits, changed);
    if let Some(child) = new {
        *root = child;
    }
}

/// Sets every voxel of `region`. Returns whether anything changed.
pub(crate) fn fill_box(
    store: &mut Store,
    root: &mut Child,
    region: &LocalBox,
    material: MaterialId,
) -> bool {
    let new = Writer { store }.fill(*root, ROOT_LEVEL, [0; 3], region, material);
    new.map(|child| *root = child).is_some()
}

/// Rewrites children through the store. Every method takes a child and the
/// level its node has (or would have, once split): 1 for a node whose
/// children are leaf records.
struct Writer<'a> {
    store: &'a mut Store,
}

impl Writer<'_> {
    /// The index and contents of the node behind `child`, if it is one.
    fn existing(&self, child: Child) -> Option<(u32, Node)> {
        match child {
            Child::Node(index) => Some((index, self.store.node(index))),
            Child::Air | Child::Uniform(_) => None,
        }
    }

    /// Stores `packed` as the new content of a child at `level` whose node,
    /// if it had one, was `existing`, and returns the new child. A node keeps
    /// its index and is rewritten only where its contents differ; content
    /// that packs to one material releases the node.
    fn write<const N: usize>(
        &mut self,
        existing: Option<(u32, Node)>,
        level: u32,
        packed: Packed<N>,
    ) -> Child {
        match (packed, existing) {
            (Packed::Filled(material), existing) => {
                if let Some((index, node)) = existing {
                    self.release(index, node, level);
                }
                Child::filled(material)
            }
            (Packed::Node(p), Some((index, old))) => {
                self.rewrite(index, old, level, p.mask, p.words());
                Child::Node(index)
            }
            (Packed::Node(p), None) => {
                let base = self.store.alloc_block(p.words());
                let node = Node {
                    mask: p.mask,
                    base,
                    info: 0,
                };
                Child::Node(self.store.alloc_node(node))
            }
        }
    }

    /// Updates node `index` from `old` to `mask` and `words`, writing only
    /// what differs. A block of the same length is overwritten in place;
    /// otherwise a new block replaces it.
    fn rewrite(&mut self, index: u32, old: Node, level: u32, mask: u64, words: &[u32]) {
        let old_len = old.block_len(level);
        let base = if old_len == words.len() {
            if self.store.block(old.base, old_len) != words {
                self.store.write_block(old.base, words);
            }
            old.base
        } else {
            let base = self.store.alloc_block(words);
            self.store.free_block(old.base, old_len);
            base
        };
        let node = Node {
            mask,
            base,
            info: 0,
        };
        if node != old {
            self.store.set_node(index, node);
        }
    }

    /// Frees one node and its block, not its children.
    fn release(&mut self, index: u32, node: Node, level: u32) {
        self.store.free_block(node.base, node.block_len(level));
        self.store.free_node(index);
    }

    /// Frees every node and palette block under and including `child`.
    fn free_subtree(&mut self, child: Child, level: u32) {
        let Some((index, node)) = self.existing(child) else {
            return;
        };
        if level == 1 {
            for record in self.records(child) {
                self.free_leaf(record);
            }
        } else {
            for grandchild in self.children(child) {
                self.free_subtree(grandchild, level - 1);
            }
        }
        self.release(index, node, level);
    }
}
