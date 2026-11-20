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
mod build;
mod fill;
mod voxel;

pub(crate) use build::{Builder, graft};
pub(crate) use fill::LocalBox;
pub(crate) use voxel::LocalEdit;

use super::ROOT_LEVEL;
use super::alloc::Store;
use std::sync::Arc;

use super::forest::{Chunk, Forest};
use super::node::{CHUNK_LEVEL, Child, Node, Packed};
use crate::consts::TREE_DEPTH;
use crate::material::MaterialId;

/// Sets voxels. `edits` must be sorted by `node::path_key` with at most one
/// edit per voxel. Calls `changed` with every edit that changed its voxel and
/// the material the voxel held before. Edits whose path meets a region that
/// is not resident are skipped; returns whether any was. A single edit that
/// is skipped changes nothing.
pub(crate) fn set_voxels(
    forest: &mut Forest,
    root: &mut Child,
    edits: &[LocalEdit],
    changed: &mut impl FnMut(LocalEdit, MaterialId),
) -> bool {
    debug_assert!(
        edits.windows(2).all(|w| {
            let key = |e: &LocalEdit| super::node::path_key(e.local, TREE_DEPTH);
            key(&w[0]) < key(&w[1])
        }),
        "voxel edits must be in tree order, one per voxel"
    );
    let mut writer = Writer::new(forest);
    let new = writer.set_voxels(*root, ROOT_LEVEL, edits, changed);
    let refused = writer.refused;
    if let Some(child) = new {
        *root = child;
    }
    refused
}

/// Sets every voxel of `region`. Returns whether anything changed.
pub(crate) fn fill_box(
    forest: &mut Forest,
    root: &mut Child,
    region: &LocalBox,
    material: MaterialId,
) -> bool {
    let new = Writer::new(forest).fill(*root, ROOT_LEVEL, [0; 3], region, material);
    new.map(|child| *root = child).is_some()
}

/// Rewrites children through the forest. Every method takes a child and the
/// level its node has (or would have, once split): 1 for a node whose
/// children are leaf records.
///
/// Above [`CHUNK_LEVEL`] the writer works on the top store; a method that
/// reaches a chunk-level child enters that child's chunk (see
/// [`in_chunk`](Self::in_chunk)) and works on its store until it returns.
struct Writer<'a> {
    forest: &'a mut Forest,
    /// The chunk being written, or `None` above the chunk level.
    chunk: Option<u32>,
    /// That chunk, checked out of the forest at its first write and put back
    /// when the writer leaves it.
    held: Option<Arc<Chunk>>,
    /// A voxel write met a region that is not resident.
    refused: bool,
}

impl<'a> Writer<'a> {
    fn new(forest: &'a mut Forest) -> Self {
        Self {
            forest,
            chunk: None,
            held: None,
            refused: false,
        }
    }
}

impl Writer<'_> {
    /// The store the writer is working on.
    #[inline]
    fn store(&self) -> &Store {
        if let Some(chunk) = &self.held {
            return &chunk.store;
        }
        match self.chunk {
            None => &self.forest.top,
            Some(id) => &self.forest.chunk(id).store,
        }
    }

    #[inline]
    fn store_mut(&mut self) -> &mut Store {
        let Some(id) = self.chunk else {
            return &mut self.forest.top;
        };
        let held = self.held.get_or_insert_with(|| self.forest.check_out(id));
        // Unique since check-out: nothing else can clone it meanwhile.
        &mut Arc::get_mut(held)
            .expect("a checked-out chunk is unique")
            .store
    }

    /// Whether `level` is the chunk level, reached from the top: the child
    /// there is a chunk id, air, uniform or not resident, and must be entered
    /// with [`in_chunk`](Self::in_chunk) before it is read or written.
    fn at_chunk_boundary(&self, level: u32) -> bool {
        level == CHUNK_LEVEL && self.chunk.is_none()
    }

    /// Runs `write` on the chunk-level child `child` inside its chunk, with
    /// the child as the chunk's store sees it. An air or uniform child gets a
    /// new chunk, which is dropped again unless the write leaves mixed
    /// content. A result of air, one material or not resident drops the
    /// chunk. Returns the child the parent should hold, as `write` does.
    fn in_chunk(
        &mut self,
        child: Child,
        write: impl FnOnce(&mut Self, Child) -> Option<Child>,
    ) -> Option<Child> {
        debug_assert!(self.at_chunk_boundary(CHUNK_LEVEL));
        debug_assert!(
            child != Child::NotResident,
            "writes never reach unloaded chunks"
        );
        let (id, fresh, local) = match child {
            Child::Node(id) => (id, false, Child::Node(self.forest.chunk(id).root)),
            filled => (self.forest.alloc_chunk(), true, filled),
        };
        self.chunk = Some(id);
        let result = write(self, local);
        self.chunk = None;
        let held = self.held.take();
        match result {
            None => {
                if let Some(chunk) = held {
                    self.forest.check_in(id, chunk);
                }
                if fresh {
                    self.forest.free_chunk(id);
                }
                None
            }
            Some(Child::Node(root)) => {
                let mut chunk = held.unwrap_or_else(|| self.forest.check_out(id));
                let generation = self.forest.next_generation();
                let c = Arc::get_mut(&mut chunk).expect("a checked-out chunk is unique");
                c.root = root;
                c.generation = generation;
                self.forest.check_in(id, chunk);
                Some(Child::Node(id))
            }
            Some(other) => {
                if let Some(chunk) = held {
                    self.forest.check_in(id, chunk);
                }
                self.forest.free_chunk(id);
                Some(other)
            }
        }
    }

    /// The index and contents of the node behind `child`, if it is one.
    #[inline]
    fn existing(&self, child: Child) -> Option<(u32, Node)> {
        match child {
            Child::Node(index) => Some((index, self.store().node(index))),
            Child::Air | Child::Uniform(_) | Child::NotResident => None,
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
            (Packed::NotResident, existing) => {
                if let Some((index, node)) = existing {
                    self.release(index, node, level);
                }
                Child::NotResident
            }
            (Packed::Node(p), Some((index, old))) => {
                self.rewrite(index, old, level, p.mask, p.info, p.words());
                Child::Node(index)
            }
            (Packed::Node(p), None) => {
                let base = self.store_mut().alloc_block(p.words());
                let node = Node {
                    mask: p.mask,
                    base,
                    info: p.info,
                };
                Child::Node(self.store_mut().alloc_node(node))
            }
        }
    }

    /// Updates node `index` from `old` to `mask` and `words`, writing only
    /// what differs. A block of the same length is overwritten in place;
    /// otherwise a new block replaces it.
    fn rewrite(&mut self, index: u32, old: Node, level: u32, mask: u64, info: u32, words: &[u32]) {
        let old_len = old.block_len(level);
        let base = if old_len == words.len() {
            if self.store().block(old.base, old_len) != words {
                self.store_mut().write_block(old.base, words);
            }
            old.base
        } else {
            let base = self.store_mut().alloc_block(words);
            self.store_mut().free_block(old.base, old_len);
            base
        };
        let node = Node { mask, base, info };
        if node != old {
            self.store_mut().set_node(index, node);
        }
    }

    /// Frees one node and its block, not its children.
    fn release(&mut self, index: u32, node: Node, level: u32) {
        self.store_mut()
            .free_block(node.base, node.block_len(level));
        self.store_mut().free_node(index);
    }

    /// Frees every node and palette block under and including `child`. A
    /// whole chunk is dropped at once.
    fn free_subtree(&mut self, child: Child, level: u32) {
        if self.at_chunk_boundary(level) {
            if let Child::Node(id) = child {
                self.forest.free_chunk(id);
            }
            return;
        }
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
