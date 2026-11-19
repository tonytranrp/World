//! The sparse 64-tree: the authoritative voxel world (decision 0004).
//!
//! [`VoxelTree`] is the single writer. [`TreeSnapshot`] is an immutable,
//! thread-safe view taken in O(1); later writes never change what it sees.
//! Both read through [`TreeRead`], whose [`NodeRef`] walk exposes the canonical
//! structure to serialization, ray traversal and the GPU mirror.
//!
//! Modules: `node` is the 16-byte node layout and leaf packing, `alloc` the
//! copy-on-write paged arenas, `mutate` the write path that keeps the tree
//! canonical.

mod alloc;
mod mutate;
mod node;

#[cfg(test)]
mod tests;

use std::fmt;

use alloc::{Storage, Store};
use mutate::LocalBox;
use node::{Child, NodeKind, bit, leaf, rank, slots};

use crate::consts::{TREE_DEPTH, TREE_FANOUT};
use crate::coords::{VoxelBox, VoxelPos, path};
use crate::edit::{DirtyBricks, Edit, EditError};
use crate::material::MaterialId;

/// The level of the root node, whose children each cover a quarter of the
/// world per axis.
const ROOT_LEVEL: u32 = TREE_DEPTH - 1;

/// Read access shared by the live tree and its snapshots.
pub trait TreeRead {
    /// The whole world as one child: air, a uniform material, or the level
    /// `TREE_DEPTH - 1` root branch.
    fn root(&self) -> NodeRef<'_>;

    /// The material of one voxel; air outside the world.
    fn get(&self, pos: VoxelPos) -> MaterialId {
        let Some(local) = path::world_local(pos) else {
            return MaterialId::AIR;
        };
        let mut node = self.root();
        loop {
            match node {
                NodeRef::Air => return MaterialId::AIR,
                NodeRef::Uniform(m) => return m,
                NodeRef::Branch(b) => node = b.child(path::child_slot(local, b.level())),
                NodeRef::Leaf(l) => return l.voxel(path::child_slot(local, 0)),
            }
        }
    }
}

/// One child in the tree, as seen by a reader.
#[derive(Clone, Copy, Debug)]
pub enum NodeRef<'a> {
    /// Every voxel of the child is air.
    Air,
    /// Every voxel of the child is this solid material.
    Uniform(MaterialId),
    /// A node of level 1 or higher with mixed children.
    Branch(BranchRef<'a>),
    /// A level-0 node holding 4x4x4 voxels, not all the same.
    Leaf(LeafRef<'a>),
}

impl<'a> NodeRef<'a> {
    /// The reader's view of a stored child whose node, if it is one, has
    /// `level`.
    fn new(storage: &'a Storage, child: Child, level: u32) -> Self {
        let index = match child {
            Child::Air => return Self::Air,
            Child::Uniform(m) => return Self::Uniform(m),
            Child::Node(index) => index,
        };
        let node = storage.node(index);
        let kind = NodeKind::at_level(level);
        let block = storage.block(node.base, node.block_len(kind));
        match kind {
            NodeKind::Leaf => Self::Leaf(LeafRef {
                mask: node.mask,
                info: node.info,
                block,
            }),
            NodeKind::Branch => Self::Branch(BranchRef {
                storage,
                level,
                mask: node.mask,
                children: block,
            }),
        }
    }
}

/// A branch node: 64 children, each covering `4^level` voxels per axis.
#[derive(Clone, Copy)]
pub struct BranchRef<'a> {
    storage: &'a Storage,
    level: u32,
    mask: u64,
    /// One word per non-air child, in slot order.
    children: &'a [u32],
}

impl<'a> BranchRef<'a> {
    /// The node's level, from 1 up to `TREE_DEPTH - 1`.
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Bit `i` is set when child slot `i` is not all air.
    pub fn mask(&self) -> u64 {
        self.mask
    }

    /// The child in `slot` (0 to 63). Panics on a slot above 63.
    pub fn child(&self, slot: u32) -> NodeRef<'a> {
        assert!(slot < TREE_FANOUT, "child slot {slot} is above 63");
        if self.mask & bit(slot) == 0 {
            return NodeRef::Air;
        }
        let word = self.children[rank(self.mask, slot)];
        NodeRef::new(self.storage, Child::from_word(word), self.level - 1)
    }

    /// The children that are not air, in ascending slot order.
    pub fn children(&self) -> impl Iterator<Item = (u32, NodeRef<'a>)> + 'a + use<'a> {
        let (storage, level) = (self.storage, self.level - 1);
        slots(self.mask)
            .zip(self.children)
            .map(move |(slot, &word)| (slot, NodeRef::new(storage, Child::from_word(word), level)))
    }
}

impl fmt::Debug for BranchRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BranchRef")
            .field("level", &self.level)
            .field("mask", &format_args!("{:#018x}", self.mask))
            .finish_non_exhaustive()
    }
}

/// A leaf node: 4x4x4 voxels with a palette of the materials present.
#[derive(Clone, Copy)]
pub struct LeafRef<'a> {
    mask: u64,
    info: u32,
    /// The palette, then the packed indices (see `node::leaf`).
    block: &'a [u32],
}

impl<'a> LeafRef<'a> {
    /// Bit `i` is set when voxel slot `i` is solid.
    pub fn mask(&self) -> u64 {
        self.mask
    }

    /// The voxel in `slot` (0 to 63). Panics on a slot above 63.
    pub fn voxel(&self, slot: u32) -> MaterialId {
        assert!(slot < TREE_FANOUT, "voxel slot {slot} is above 63");
        if self.mask & bit(slot) == 0 {
            return MaterialId::AIR;
        }
        let index = leaf::packed_index(self.block, self.info, rank(self.mask, slot));
        leaf::palette_entry(self.block, index)
    }

    /// The solid materials present, ascending by id, each once.
    pub fn palette(&self) -> impl Iterator<Item = MaterialId> + 'a + use<'a> {
        let block = self.block;
        (0..leaf::palette_len(self.info)).map(move |i| leaf::palette_entry(block, i))
    }

    /// Bits per packed palette index: the smallest of 0, 1, 2, 4 or 8 that
    /// indexes the palette, so 0 when it has one material.
    pub fn index_bits(&self) -> u32 {
        leaf::index_bits(self.info)
    }

    /// The packed palette indices exactly as stored: one per solid voxel in
    /// slot order, [`index_bits`](Self::index_bits) wide, LSB-first in whole
    /// words. Empty when the width is 0.
    pub fn index_words(&self) -> &'a [u32] {
        &self.block[leaf::palette_words(leaf::palette_len(self.info))..]
    }
}

impl fmt::Debug for LeafRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeafRef")
            .field("mask", &format_args!("{:#018x}", self.mask))
            .field("palette", &self.palette().map(|m| m.0).collect::<Vec<_>>())
            .finish()
    }
}

/// Storage figures, for budgets, benchmarks and leak tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeStats {
    /// Live branch nodes.
    pub branches: usize,
    /// Live leaf nodes.
    pub leaves: usize,
    /// Live words: packed children, palettes and indices.
    pub words: usize,
    /// Node slots on the free list.
    pub free_nodes: usize,
    /// Words in recycled blocks.
    pub free_words: usize,
    /// Bytes held by the writer's pages, live or free.
    pub bytes: usize,
}

/// The live, editable world tree. A new tree is all air.
///
/// Every write leaves the tree canonical, so equal content always has equal
/// structure and equal live [`stats`](Self::stats).
#[derive(Default)]
pub struct VoxelTree {
    store: Store,
    root: Child,
}

impl VoxelTree {
    /// An all-air tree. Allocates nothing until the first solid voxel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets one voxel. Returns whether its material changed.
    pub fn set(&mut self, pos: VoxelPos, material: MaterialId) -> Result<bool, EditError> {
        let local = path::world_local(pos).ok_or(EditError::OutOfWorld { pos })?;
        Ok(mutate::set_voxel(
            &mut self.store,
            &mut self.root,
            local,
            material,
        ))
    }

    /// Sets every voxel of `region`. Returns whether anything changed. Fails,
    /// changing nothing, if any part of a non-empty region is outside the world.
    pub fn fill_box(&mut self, region: VoxelBox, material: MaterialId) -> Result<bool, EditError> {
        let Some(region) = LocalBox::of(region)? else {
            return Ok(false);
        };
        Ok(mutate::fill_box(
            &mut self.store,
            &mut self.root,
            &region,
            material,
        ))
    }

    /// Applies a batch in order: all of it, or none of it if any edit is
    /// outside the world. Returns the bricks whose content changed.
    ///
    /// The report compares each brick before and after the whole batch, so a
    /// voxel changed and then restored within the batch reports nothing.
    pub fn apply(&mut self, edits: &[Edit]) -> Result<DirtyBricks, EditError> {
        if let Some(outside) = edits.iter().find(|e| !e.pos.in_world()) {
            return Err(EditError::OutOfWorld { pos: outside.pos });
        }
        // The material of every voxel the batch changed, as it was before.
        let mut before = Vec::new();
        for edit in edits {
            let old = self.get(edit.pos);
            let local = path::world_local(edit.pos).expect("every edit was checked above");
            if mutate::set_voxel(&mut self.store, &mut self.root, local, edit.material) {
                before.push((edit.pos, old));
            }
        }
        // A stable sort keeps each voxel's first change, the true original.
        before.sort_by_key(|&(pos, _)| pos);
        before.dedup_by_key(|&mut (pos, _)| pos);
        let changed = before
            .into_iter()
            .filter(|&(pos, old)| self.get(pos) != old)
            .map(|(pos, _)| pos.brick())
            .collect();
        Ok(DirtyBricks::from_unsorted(changed))
    }

    /// An immutable view of the tree as it is now. O(1).
    pub fn snapshot(&self) -> TreeSnapshot {
        TreeSnapshot {
            storage: self.store.storage().clone(),
            root: self.root,
        }
    }

    /// Live and free storage. The live counts depend only on the content.
    pub fn stats(&self) -> TreeStats {
        self.store.stats()
    }
}

impl TreeRead for VoxelTree {
    fn root(&self) -> NodeRef<'_> {
        NodeRef::new(self.store.storage(), self.root, ROOT_LEVEL)
    }
}

impl fmt::Debug for VoxelTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VoxelTree")
            .field("root", &self.root)
            .field("stats", &self.stats())
            .finish()
    }
}

/// An immutable view of a [`VoxelTree`] at one moment. Cheap to clone and
/// safe to send to other threads.
#[derive(Clone)]
pub struct TreeSnapshot {
    storage: Storage,
    root: Child,
}

impl TreeRead for TreeSnapshot {
    fn root(&self) -> NodeRef<'_> {
        NodeRef::new(&self.storage, self.root, ROOT_LEVEL)
    }
}

impl fmt::Debug for TreeSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeSnapshot")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Whether two trees hold the same voxels. Trees are canonical, so this is a
/// structural walk that never visits uniform regions voxel by voxel.
pub fn same_content(a: &impl TreeRead, b: &impl TreeRead) -> bool {
    same_child(a.root(), b.root())
}

fn same_child(a: NodeRef<'_>, b: NodeRef<'_>) -> bool {
    match (a, b) {
        (NodeRef::Air, NodeRef::Air) => true,
        (NodeRef::Uniform(x), NodeRef::Uniform(y)) => x == y,
        (NodeRef::Branch(x), NodeRef::Branch(y)) => {
            x.level() == y.level()
                && x.mask() == y.mask()
                && x.children()
                    .zip(y.children())
                    .all(|((_, cx), (_, cy))| same_child(cx, cy))
        }
        (NodeRef::Leaf(x), NodeRef::Leaf(y)) => {
            x.mask() == y.mask() && (0..64).all(|slot| x.voxel(slot) == y.voxel(slot))
        }
        _ => false,
    }
}

impl PartialEq for VoxelTree {
    fn eq(&self, other: &Self) -> bool {
        same_content(self, other)
    }
}

impl Eq for VoxelTree {}

impl PartialEq for TreeSnapshot {
    fn eq(&self, other: &Self) -> bool {
        same_content(self, other)
    }
}

impl Eq for TreeSnapshot {}

const _: () = assert!(TREE_DEPTH >= 2);

// Snapshots are handed to render and streaming threads (decision 0003).
const _: () = {
    const fn send_and_sync<T: Send + Sync>() {}
    send_and_sync::<TreeSnapshot>();
};
