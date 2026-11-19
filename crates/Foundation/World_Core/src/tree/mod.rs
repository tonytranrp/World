//! The sparse 64-tree: the authoritative voxel world (decision 0004).
//!
//! [`VoxelTree`] is the single writer. [`TreeSnapshot`] is an immutable,
//! thread-safe view taken in O(1); later writes never change what it sees.
//! Both read through [`TreeRead`], whose [`NodeRef`] walk exposes the canonical
//! structure to serialization, ray traversal and the GPU mirror.
//!
//! Modules: `node` is the storage layout (16-byte nodes, inline leaf records
//! and palette blocks), `alloc` the copy-on-write paged arenas, `mutate` the
//! write path that keeps the tree canonical.

mod alloc;
mod mutate;
pub mod node;

#[cfg(test)]
mod tests;

use std::fmt;

pub use alloc::{NODE_PAGE_LEN, WORD_PAGE_LEN};
pub use node::{Node, UNIFORM_BIT};

use alloc::{Storage, Store};
use mutate::{LocalBox, LocalEdit};
use node::leaf::{self, Record};
use node::{Child, RECORD_WORDS, bit, path_key, rank, slots};

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
    /// `level` (1 or higher).
    fn new(storage: &'a Storage, child: Child, level: u32) -> Self {
        match child {
            Child::Air => Self::Air,
            Child::Uniform(m) => Self::Uniform(m),
            Child::Node(index) => {
                let node = storage.node(index);
                Self::Branch(BranchRef {
                    storage,
                    level,
                    mask: node.mask,
                    block: storage.block(node.base, node.block_len(level)),
                })
            }
        }
    }

    /// The reader's view of a stored leaf record.
    fn leaf(storage: &'a Storage, record: Record) -> Self {
        if let Some(m) = record.uniform() {
            return Self::Uniform(m);
        }
        let content = match record.palette_base() {
            Some(base) => {
                let len = leaf::block_len(record.mask, storage.word(base));
                LeafContent::Palette(storage.block(base, len))
            }
            None => LeafContent::Single(
                record
                    .material()
                    .expect("a record without a palette has one material"),
            ),
        };
        Self::Leaf(LeafRef {
            mask: record.mask,
            content,
        })
    }
}

/// A branch node: 64 children, each covering `4^level` voxels per axis.
#[derive(Clone, Copy)]
pub struct BranchRef<'a> {
    storage: &'a Storage,
    level: u32,
    mask: u64,
    /// One child word (level 2 and up) or leaf record (level 1) per non-air
    /// child, in slot order.
    block: &'a [u32],
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
        self.child_at(rank(self.mask, slot))
    }

    /// The children that are not air, in ascending slot order.
    pub fn children(&self) -> impl Iterator<Item = (u32, NodeRef<'a>)> + 'a + use<'a> {
        let branch = *self;
        slots(self.mask)
            .enumerate()
            .map(move |(k, slot)| (slot, branch.child_at(k)))
    }

    /// The `k`-th non-air child.
    fn child_at(&self, k: usize) -> NodeRef<'a> {
        if self.level == 1 {
            let at = k * RECORD_WORDS;
            NodeRef::leaf(
                self.storage,
                Record::read(&self.block[at..at + RECORD_WORDS]),
            )
        } else {
            NodeRef::new(
                self.storage,
                Child::from_word(self.block[k]),
                self.level - 1,
            )
        }
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

/// A leaf: 4x4x4 voxels, not all the same, with the materials present.
#[derive(Clone, Copy)]
pub struct LeafRef<'a> {
    mask: u64,
    content: LeafContent<'a>,
}

/// What a leaf's solid voxels are.
#[derive(Clone, Copy)]
enum LeafContent<'a> {
    /// All one material: the record alone.
    Single(MaterialId),
    /// The palette block: header, palette, packed indices (see `node`).
    Palette(&'a [u32]),
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
        match self.content {
            LeafContent::Single(m) => m,
            LeafContent::Palette(block) => leaf::solid_voxel(block, rank(self.mask, slot)),
        }
    }

    /// The solid materials present, ascending by id, each once.
    pub fn palette(&self) -> impl Iterator<Item = MaterialId> + 'a + use<'a> {
        let content = self.content;
        let len = match content {
            LeafContent::Single(_) => 1,
            LeafContent::Palette(block) => leaf::palette_len(block[0]),
        };
        (0..len).map(move |i| match content {
            LeafContent::Single(m) => m,
            LeafContent::Palette(block) => leaf::palette_entry(block, i),
        })
    }

    /// Bits per packed palette index: the smallest of 0, 1, 2, 4 or 8 that
    /// indexes the palette, so 0 when it has one material.
    pub fn index_bits(&self) -> u32 {
        match self.content {
            LeafContent::Single(_) => 0,
            LeafContent::Palette(block) => leaf::index_bits(block[0]),
        }
    }

    /// The packed palette indices exactly as stored: one per solid voxel in
    /// slot order, [`index_bits`](Self::index_bits) wide, LSB-first in whole
    /// words. Empty when the width is 0.
    pub fn index_words(&self) -> &'a [u32] {
        match self.content {
            LeafContent::Single(_) => &[],
            LeafContent::Palette(block) => {
                &block[1 + leaf::palette_words(leaf::palette_len(block[0]))..]
            }
        }
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
    /// Live nodes, all branches of level 1 and up.
    pub branches: usize,
    /// Live leaves: 4x4x4 children of level-1 nodes that are neither air
    /// nor uniform. They are records inside their parent's block, not nodes.
    pub leaves: usize,
    /// Live words: child words, leaf records and palette blocks.
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
        let mut changed = false;
        let edit = LocalEdit { local, material };
        mutate::set_voxels(&mut self.store, &mut self.root, &[edit], &mut |_, _| {
            changed = true;
        });
        Ok(changed)
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
    /// Later edits of the same voxel win. The batch is written in tree order,
    /// so every node on its paths is rebuilt once, however many edits fall
    /// under it.
    pub fn apply(&mut self, edits: &[Edit]) -> Result<DirtyBricks, EditError> {
        let mut sorted = Vec::with_capacity(edits.len());
        for edit in edits {
            let pos = edit.pos;
            let local = path::world_local(pos).ok_or(EditError::OutOfWorld { pos })?;
            let edit = LocalEdit {
                local,
                material: edit.material,
            };
            sorted.push((path_key(local, TREE_DEPTH), edit));
        }
        // A stable sort keeps the batch order among edits of one voxel; only
        // the last of them decides the voxel's final material.
        sorted.sort_by_key(|&(key, _)| key);
        sorted.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1 = later.1;
            }
            same
        });
        let edits: Vec<LocalEdit> = sorted.into_iter().map(|(_, edit)| edit).collect();

        // Each voxel is written once with its final material, so a change is
        // a net change of the batch.
        let mut bricks = Vec::new();
        mutate::set_voxels(&mut self.store, &mut self.root, &edits, &mut |edit, _| {
            let brick = path::from_world_local(edit.local).brick();
            if bricks.last() != Some(&brick) {
                bricks.push(brick);
            }
        });
        Ok(DirtyBricks::from_unsorted(bricks))
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

impl TreeSnapshot {
    /// The root as a child word (see [`node`]): `None` when the whole world
    /// is air, a word with [`UNIFORM_BIT`] when it is one material, else the
    /// index of the root node.
    pub fn root_word(&self) -> Option<u32> {
        self.root.word()
    }

    /// The node pages with their page index, for uploads that cast them to
    /// bytes (`bytemuck::cast_slice`) without copying. Page `i` holds nodes
    /// `i * NODE_PAGE_LEN ..`; slots not reachable from the root are free and
    /// hold stale data.
    pub fn node_pages(&self) -> impl Iterator<Item = (usize, &[Node])> {
        self.storage.node_pages().enumerate()
    }

    /// The word pages with their page index, like [`node_pages`](Self::node_pages).
    /// Page `i` holds words `i * WORD_PAGE_LEN ..`.
    pub fn word_pages(&self) -> impl Iterator<Item = (usize, &[u32])> {
        self.storage.word_pages().enumerate()
    }
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
