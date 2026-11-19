//! The only write path into the tree (decision 0004).
//!
//! A write recurses from the root down the children it affects and rebuilds
//! each changed node bottom-up. Where a write lands inside an air or uniform
//! child, that child is split into a node; every rebuilt node is repacked
//! canonically ([`Packed`]), and one whose content turns out to be all one
//! material is released and becomes a filled child. Collapse is decided
//! locally: children are canonical by induction, so a node is uniform exactly
//! when its 64 voxels (leaf) or 64 child words (branch) are all the same
//! material. Released nodes and their subtrees go back to the allocator.
//!
//! Each step returns `None` when its child did not change, so a write that
//! changes nothing allocates, copies and reports nothing. A node rebuilt in
//! place keeps its index, so its parent's word, and every page above it, is
//! left alone. A single-voxel write visits one child per level; a box fill
//! visits only partially covered children and replaces covered ones whole.

mod branch;
mod fill;
mod voxel;

pub(crate) use fill::LocalBox;

use super::ROOT_LEVEL;
use super::alloc::Store;
use super::node::leaf::{self, Voxels};
use super::node::{Child, Node, NodeKind, Packed, PackedNode};
use crate::material::MaterialId;

/// Sets the voxel at world-local `local`. Returns whether it changed.
pub(crate) fn set_voxel(
    store: &mut Store,
    root: &mut Child,
    local: [u32; 3],
    material: MaterialId,
) -> bool {
    let new = Writer { store }.set_voxel(*root, ROOT_LEVEL, local, material);
    replace_root(root, new)
}

/// Sets every voxel of `region`. Returns whether anything changed.
pub(crate) fn fill_box(
    store: &mut Store,
    root: &mut Child,
    region: &LocalBox,
    material: MaterialId,
) -> bool {
    let new = Writer { store }.fill(*root, ROOT_LEVEL, [0; 3], region, material);
    replace_root(root, new)
}

fn replace_root(root: &mut Child, new: Option<Child>) -> bool {
    new.map(|child| *root = child).is_some()
}

/// Rewrites children through the store. Every method takes a child and the
/// level its node has (or would have, once split): 0 for a leaf.
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

    /// The 64 voxels of a leaf-level child.
    fn leaf_voxels(&self, child: Child) -> Voxels {
        match child {
            Child::Air => [MaterialId::AIR; 64],
            Child::Uniform(m) => [m; 64],
            Child::Node(index) => {
                let node = self.store.node(index);
                let block = self.store.block(node.base, node.block_len(NodeKind::Leaf));
                leaf::decode(node.mask, node.info, block)
            }
        }
    }

    /// Stores `packed` as the new content of a child whose node, if it had
    /// one, was `existing`, and returns the new child. A node keeps its index
    /// and is rewritten only where its contents differ; content that packs to
    /// one material releases the node.
    fn write(&mut self, existing: Option<(u32, Node)>, kind: NodeKind, packed: Packed) -> Child {
        match (packed, existing) {
            (Packed::Filled(material), existing) => {
                if let Some((index, node)) = existing {
                    self.release(index, node, kind);
                }
                Child::filled(material)
            }
            (Packed::Node(p), Some((index, old))) => {
                self.rewrite(index, old, kind, &p);
                Child::Node(index)
            }
            (Packed::Node(p), None) => {
                let base = self.store.alloc_block(p.words());
                let node = Node {
                    mask: p.mask,
                    base,
                    info: p.info,
                };
                Child::Node(self.store.alloc_node(kind, node))
            }
        }
    }

    /// Updates node `index` from `old` to `p`, writing only what differs. A
    /// block of the same length is overwritten in place; otherwise a new
    /// block replaces it.
    fn rewrite(&mut self, index: u32, old: Node, kind: NodeKind, p: &PackedNode) {
        let old_len = old.block_len(kind);
        let base = if old_len == p.words().len() {
            if self.store.block(old.base, old_len) != p.words() {
                self.store.write_block(old.base, p.words());
            }
            old.base
        } else {
            let base = self.store.alloc_block(p.words());
            self.store.free_block(old.base, old_len);
            base
        };
        let node = Node {
            mask: p.mask,
            base,
            info: p.info,
        };
        if node != old {
            self.store.set_node(index, node);
        }
    }

    /// Frees one node and its block, not its children.
    fn release(&mut self, index: u32, node: Node, kind: NodeKind) {
        self.store.free_block(node.base, node.block_len(kind));
        self.store.free_node(kind, index);
    }

    /// Frees every node under and including `child`.
    fn free_subtree(&mut self, child: Child, level: u32) {
        let Some((index, node)) = self.existing(child) else {
            return;
        };
        let kind = NodeKind::at_level(level);
        if kind == NodeKind::Branch {
            for grandchild in self.children(child) {
                self.free_subtree(grandchild, level - 1);
            }
        }
        self.release(index, node, kind);
    }
}
