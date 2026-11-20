//! Reading voxels.
//!
//! Two layers. [`VoxelSource`] and [`NodeView`] are the generic interface
//! that the world tree and the dense grids of dynamic objects share, so ray
//! traversal, physics queries, ambient occlusion, comparison and checks are
//! written once and monomorphized for each storage shape. [`TreeRead`] is the
//! world tree's own view, whose [`NodeRef`] exposes the stored structure
//! (palettes, index widths) to serialization and tests; every `TreeRead` is a
//! `VoxelSource`.
//!
//! Point lookups on the world tree skip the views entirely: they read node
//! and word values straight from storage, and [`TreeCursor`](super::TreeCursor)
//! reuses the path of its previous lookup.

use std::fmt;

use super::alloc::{Storage, StorageReader};
use super::forest::ChunkLookup;
use super::mutate::LocalBox;
use super::node::leaf::{self, Record};
use super::node::{
    CHUNK_LEVEL, Child, NOT_RESIDENT, RECORD_WORDS, UNIFORM_BIT, bit, rank, slot_at, slots,
};
use super::{ROOT_LEVEL, WORLD_ORIGIN};
use crate::consts::{TREE_DEPTH, TREE_FANOUT};
use crate::coords::{VoxelPos, path};
use crate::material::MaterialId;

/// Anything that reads as a cube of `4^levels` voxels per axis, organized as
/// a canonical 64-tree: the world tree, its snapshots, and the dense grids of
/// dynamic objects.
///
/// The root stands where a node of level `levels - 1` would; the children of
/// a level-`L` node each cover `4^L` voxels per axis, and level-0 nodes are
/// leaves of 4x4x4 voxels. Child slot `x + 4y + 16z` takes the two bits at
/// `2L` of each axis of the position relative to [`origin`](Self::origin).
pub trait VoxelSource {
    /// The view of one child.
    type Node<'a>: NodeView
    where
        Self: 'a;

    /// Node levels: 10 for the world tree, 3 for a 64-voxel grid.
    fn levels(&self) -> u32;

    /// The lowest voxel of the cube.
    fn origin(&self) -> VoxelPos;

    /// The whole cube as one child.
    fn root(&self) -> Self::Node<'_>;

    /// The material of one voxel; air outside the cube. Sources override this
    /// with a direct lookup.
    fn get(&self, pos: VoxelPos) -> MaterialId {
        descend(self.root(), self.levels(), self.origin(), pos)
    }
}

/// What a child is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// Every voxel is air.
    Air,
    /// Every voxel is this solid material.
    Uniform(MaterialId),
    /// A node of level 1 or higher with mixed children.
    Branch,
    /// A level-0 node: 4x4x4 voxels, not all the same.
    Leaf,
    /// The content is not loaded (decision 0006). Only children at the chunk
    /// level and above of the world tree are ever not resident.
    NotResident,
}

/// One child of a [`VoxelSource`], at a level its caller tracks.
///
/// Views are canonical: a child whose voxels are all air is
/// [`NodeKind::Air`] and one whose voxels are all one solid material is
/// [`NodeKind::Uniform`], whatever the storage holds, so equal content gives
/// equal views. A [`NodeKind::NotResident`] child has mask 0, reads as air
/// and is its own child, like air, but its content is unknown.
pub trait NodeView: Copy {
    fn kind(&self) -> NodeKind;

    /// Bit `i` is set when child (branch) or voxel (leaf) `i` is not air.
    /// Air is 0, uniform is all ones.
    fn mask(&self) -> u64;

    /// The child in `slot` (0 to 63). Air and uniform children return
    /// themselves, which is what every one of their children is. Panics for
    /// a leaf.
    fn child(&self, slot: u32) -> Self;

    /// The voxel in `slot` (0 to 63) of a leaf; the material of an air or
    /// uniform child. Panics for a branch.
    fn voxel(&self, slot: u32) -> MaterialId;

    /// The distinct solid materials of a leaf, ascending; one for a uniform
    /// child and none for air. Panics for a branch.
    fn palette(&self) -> Palette {
        Palette::from_voxels((0..TREE_FANOUT).map(|slot| self.voxel(slot)))
    }

    /// The children that are not air, in ascending slot order.
    fn children(&self) -> impl Iterator<Item = (u32, Self)> {
        let node = *self;
        slots(self.mask()).map(move |slot| (slot, node.child(slot)))
    }
}

/// The materials of a leaf, as [`NodeView::palette`] returns them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    entries: [MaterialId; TREE_FANOUT as usize],
    len: usize,
}

impl Palette {
    /// The distinct solid materials among `voxels`, ascending.
    pub fn from_voxels(voxels: impl IntoIterator<Item = MaterialId>) -> Self {
        let mut palette = Self::stored(voxels.into_iter().filter(|m| !m.is_air()));
        let entries = &mut palette.entries[..palette.len];
        entries.sort_unstable();
        let mut unique = 0;
        for i in 0..entries.len() {
            if unique == 0 || entries[unique - 1] != entries[i] {
                entries[unique] = entries[i];
                unique += 1;
            }
        }
        palette.len = unique;
        palette
    }

    /// Up to 64 entries exactly as given, so checks can see a stored palette
    /// as it is.
    pub(crate) fn stored(entries: impl IntoIterator<Item = MaterialId>) -> Self {
        let mut palette = Self {
            entries: [MaterialId::AIR; TREE_FANOUT as usize],
            len: 0,
        };
        for m in entries {
            palette.entries[palette.len] = m;
            palette.len += 1;
        }
        palette
    }

    pub fn as_slice(&self) -> &[MaterialId] {
        &self.entries[..self.len]
    }
}

impl fmt::Debug for Palette {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

/// The voxel at `pos` found by descending views from `root`, the root of a
/// `levels`-level cube at `origin`.
pub(crate) fn descend<N: NodeView>(
    root: N,
    levels: u32,
    origin: VoxelPos,
    pos: VoxelPos,
) -> MaterialId {
    let size = 1i64 << (2 * levels);
    let offset = |p: i32, o: i32| {
        let d = i64::from(p) - i64::from(o);
        (0..size).contains(&d).then_some(d as u32)
    };
    let (Some(x), Some(y), Some(z)) = (
        offset(pos.x, origin.x),
        offset(pos.y, origin.y),
        offset(pos.z, origin.z),
    ) else {
        return MaterialId::AIR;
    };
    let local = [x, y, z];
    let mut node = root;
    let mut level = levels - 1;
    loop {
        match node.kind() {
            NodeKind::Air | NodeKind::NotResident => return MaterialId::AIR,
            NodeKind::Uniform(m) => return m,
            NodeKind::Leaf => return node.voxel(slot_at(local, 0)),
            NodeKind::Branch => {
                node = node.child(slot_at(local, level));
                level -= 1;
            }
        }
    }
}

/// Whether two sources hold the same voxels at the same place. Views are
/// canonical, so this is a structural walk that never visits uniform regions
/// voxel by voxel.
pub fn same_content(a: &impl VoxelSource, b: &impl VoxelSource) -> bool {
    a.levels() == b.levels() && a.origin() == b.origin() && same_node(a.root(), b.root())
}

/// Whether two views of children at the same level hold the same voxels.
pub(crate) fn same_node(a: impl NodeView, b: impl NodeView) -> bool {
    match (a.kind(), b.kind()) {
        (NodeKind::Air, NodeKind::Air) | (NodeKind::NotResident, NodeKind::NotResident) => true,
        (NodeKind::Uniform(x), NodeKind::Uniform(y)) => x == y,
        (NodeKind::Branch, NodeKind::Branch) => {
            a.mask() == b.mask()
                && a.children()
                    .zip(b.children())
                    .all(|((_, x), (_, y))| same_node(x, y))
        }
        (NodeKind::Leaf, NodeKind::Leaf) => {
            a.mask() == b.mask() && (0..TREE_FANOUT).all(|slot| a.voxel(slot) == b.voxel(slot))
        }
        _ => false,
    }
}

/// The world tree's own read access, shared by the live tree and its
/// snapshots. Every `TreeRead` is a [`VoxelSource`] of `TREE_DEPTH` levels
/// covering the world.
///
/// Both traits name `root` and `get`. The tree and snapshot types also have
/// them as inherent methods, so calls on them are never ambiguous; generic
/// code bounded by `TreeRead` with `VoxelSource` in scope calls
/// `TreeRead::get(&tree, pos)`.
pub trait TreeRead {
    /// The whole world as one child: air, a uniform material, or the level
    /// `TREE_DEPTH - 1` root branch.
    fn root(&self) -> NodeRef<'_>;

    /// The material of one voxel; air outside the world.
    fn get(&self, pos: VoxelPos) -> MaterialId {
        descend(self.root(), TREE_DEPTH, WORLD_ORIGIN, pos)
    }
}

impl<T: TreeRead + ?Sized> VoxelSource for T {
    type Node<'a>
        = NodeRef<'a>
    where
        T: 'a;

    fn levels(&self) -> u32 {
        TREE_DEPTH
    }

    fn origin(&self) -> VoxelPos {
        WORLD_ORIGIN
    }

    fn root(&self) -> NodeRef<'_> {
        TreeRead::root(self)
    }

    fn get(&self, pos: VoxelPos) -> MaterialId {
        TreeRead::get(self, pos)
    }
}

/// One child in the tree, as stored.
#[derive(Clone, Copy, Debug)]
pub enum NodeRef<'a> {
    /// Every voxel of the child is air.
    Air,
    /// Every voxel of the child is this solid material.
    Uniform(MaterialId),
    /// A node of level 1 or higher with mixed children.
    Branch(BranchRef<'a>),
    /// A leaf of 4x4x4 voxels, not all the same.
    Leaf(LeafRef<'a>),
    /// The child's content is not loaded.
    NotResident,
}

impl<'a> NodeRef<'a> {
    /// The reader's view of a stored child whose node, if it is one, has
    /// `level` (1 or higher) and lives in `storage`.
    pub(crate) fn new(
        storage: &'a Storage,
        chunks: &'a dyn ChunkLookup,
        child: Child,
        level: u32,
    ) -> Self {
        match child {
            Child::Air => Self::Air,
            Child::Uniform(m) => Self::Uniform(m),
            Child::NotResident => Self::NotResident,
            Child::Node(index) => {
                let node = storage.node(index);
                Self::Branch(BranchRef {
                    storage,
                    chunks,
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

impl NodeView for NodeRef<'_> {
    fn kind(&self) -> NodeKind {
        match self {
            Self::Air => NodeKind::Air,
            Self::Uniform(m) => NodeKind::Uniform(*m),
            Self::Branch(_) => NodeKind::Branch,
            Self::Leaf(_) => NodeKind::Leaf,
            Self::NotResident => NodeKind::NotResident,
        }
    }

    fn mask(&self) -> u64 {
        match self {
            Self::Air | Self::NotResident => 0,
            Self::Uniform(_) => u64::MAX,
            Self::Branch(b) => b.mask(),
            Self::Leaf(l) => l.mask(),
        }
    }

    fn child(&self, slot: u32) -> Self {
        match self {
            Self::Air | Self::Uniform(_) | Self::NotResident => *self,
            Self::Branch(b) => b.child(slot),
            Self::Leaf(_) => panic!("a leaf has voxels, not children"),
        }
    }

    fn voxel(&self, slot: u32) -> MaterialId {
        match self {
            Self::Air | Self::NotResident => MaterialId::AIR,
            Self::Uniform(m) => *m,
            Self::Leaf(l) => l.voxel(slot),
            Self::Branch(_) => panic!("a branch has children, not voxels"),
        }
    }

    /// The stored palette, exactly as stored.
    fn palette(&self) -> Palette {
        match self {
            Self::Leaf(l) => Palette::stored(l.palette()),
            Self::Air | Self::NotResident => Palette::stored([]),
            Self::Uniform(m) => Palette::stored([*m]),
            Self::Branch(_) => panic!("a branch has children, not a palette"),
        }
    }
}

/// A branch node: 64 children, each covering `4^level` voxels per axis.
#[derive(Clone, Copy)]
pub struct BranchRef<'a> {
    storage: &'a Storage,
    chunks: &'a dyn ChunkLookup,
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
            let child = Child::from_word(self.block[k]);
            if self.level == CHUNK_LEVEL + 1
                && let Child::Node(id) = child
            {
                // The children of a level-5 node root chunks.
                let (storage, root) = self.chunks.chunk_root(id);
                return NodeRef::new(storage, self.chunks, Child::Node(root), CHUNK_LEVEL);
            }
            NodeRef::new(self.storage, self.chunks, child, self.level - 1)
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

/// The storage and root of a tree or snapshot: what point lookups read.
#[derive(Clone, Copy)]
pub(crate) struct TreeView<'a> {
    pub top: &'a Storage,
    pub chunks: &'a dyn ChunkLookup,
    pub root: Child,
}

impl<'a> TreeView<'a> {
    pub fn root(self) -> NodeRef<'a> {
        NodeRef::new(self.top, self.chunks, self.root, ROOT_LEVEL)
    }

    /// The material of one voxel; air outside the world and where the region
    /// is not resident.
    pub fn get(self, pos: VoxelPos) -> MaterialId {
        self.lookup(pos).unwrap_or(MaterialId::AIR)
    }

    /// The material of one voxel, read straight from storage: one node and
    /// one word per level, no views; `None` where the region is not
    /// resident. The readers remember the last node and word page, which the
    /// nodes of one path often share.
    pub fn lookup(self, pos: VoxelPos) -> Option<MaterialId> {
        let Some(local) = path::world_local(pos) else {
            return Some(MaterialId::AIR);
        };
        let mut index = match self.root {
            Child::Air => return Some(MaterialId::AIR),
            Child::Uniform(m) => return Some(m),
            Child::NotResident => return None,
            Child::Node(index) => index,
        };
        let mut storage = self.top.reader();
        let mut level = ROOT_LEVEL;
        loop {
            match step(&mut storage, index, level, local) {
                Step::Filled(m) => return Some(m),
                Step::NotResident => return None,
                Step::Node(child) => {
                    level -= 1;
                    index = child;
                    if level == CHUNK_LEVEL {
                        let (chunk, root) = self.chunks.chunk_root(child);
                        storage = chunk.reader();
                        index = root;
                    }
                }
                Step::Record(at) => {
                    return Some(record_voxel(&mut storage, at, slot_at(local, 0)));
                }
            }
        }
    }

    /// The chunk id stored at the chunk-level position holding world-local
    /// `local`, if that region is a chunk (mixed and resident).
    pub fn chunk_id(self, local: [u32; 3]) -> Option<u32> {
        let mut index = match self.root {
            Child::Node(index) => index,
            Child::Air | Child::Uniform(_) | Child::NotResident => return None,
        };
        let mut storage = self.top.reader();
        let mut level = ROOT_LEVEL;
        loop {
            match step(&mut storage, index, level, local) {
                Step::Node(child) if level == CHUNK_LEVEL + 1 => return Some(child),
                Step::Node(child) => {
                    index = child;
                    level -= 1;
                }
                Step::Filled(_) | Step::NotResident | Step::Record(_) => return None,
            }
        }
    }

    /// Whether the region holding world-local `local` is loaded. Walks the
    /// top levels only.
    pub fn resident(self, local: [u32; 3]) -> bool {
        let mut index = match self.root {
            Child::Node(index) => index,
            Child::NotResident => return false,
            Child::Air | Child::Uniform(_) => return true,
        };
        let mut storage = self.top.reader();
        let mut level = ROOT_LEVEL;
        while level > CHUNK_LEVEL {
            match step(&mut storage, index, level, local) {
                Step::NotResident => return false,
                Step::Node(child) => {
                    index = child;
                    level -= 1;
                }
                Step::Filled(_) | Step::Record(_) => return true,
            }
        }
        true
    }

    /// A world-local voxel of `region` whose region is not resident, if
    /// there is one.
    pub fn unloaded_in(self, region: &LocalBox) -> Option<[u32; 3]> {
        unloaded_in(self.root(), ROOT_LEVEL, [0; 3], region)
    }
}

/// A voxel of `region` inside `node`, a child at `level` whose lowest voxel
/// is `origin`, that is not resident.
fn unloaded_in(
    node: NodeRef<'_>,
    level: u32,
    origin: [u32; 3],
    region: &LocalBox,
) -> Option<[u32; 3]> {
    match node {
        NodeRef::NotResident => Some(region.lowest_in(origin)),
        NodeRef::Branch(b) if level > CHUNK_LEVEL => {
            let span = path::child_span(level);
            region.slots(origin, span).find_map(|slot| {
                let offset = path::slot_offset(slot);
                let child_origin = [0, 1, 2].map(|a| origin[a] + offset[a] * span);
                unloaded_in(b.child(slot), level - 1, child_origin, region)
            })
        }
        _ => None,
    }
}

/// Where one level of a point lookup leads.
pub(crate) enum Step {
    /// An air or uniform child: the voxel's material.
    Filled(MaterialId),
    /// A child whose content is not loaded.
    NotResident,
    /// The child word one level down: a node index, or a chunk id when the
    /// step was taken at level `CHUNK_LEVEL + 1`.
    Node(u32),
    /// The word offset of the leaf record that holds the voxel.
    Record(u32),
}

/// The child of node `index`, at `level`, that holds local coordinate `local`.
#[inline]
pub(crate) fn step(
    storage: &mut StorageReader<'_>,
    index: u32,
    level: u32,
    local: [u32; 3],
) -> Step {
    let node = storage.node(index);
    let slot = slot_at(local, level);
    if node.mask & bit(slot) == 0 {
        return Step::Filled(MaterialId::AIR);
    }
    let k = rank(node.mask, slot) as u32;
    if level == 1 {
        return Step::Record(node.base + RECORD_WORDS as u32 * k);
    }
    let word = storage.word(node.base + k);
    if word == NOT_RESIDENT {
        Step::NotResident
    } else if word & UNIFORM_BIT != 0 {
        Step::Filled(MaterialId(word as u16))
    } else {
        Step::Node(word)
    }
}

/// Voxel `slot` of the leaf whose record is at word `at`.
#[inline]
pub(crate) fn record_voxel(storage: &mut StorageReader<'_>, at: u32, slot: u32) -> MaterialId {
    let record = Record::read(storage.block(at, RECORD_WORDS));
    if record.mask & bit(slot) == 0 {
        return MaterialId::AIR;
    }
    match record.material() {
        Some(m) => m,
        // A block never straddles a page, so the rest of the page holds it.
        None => leaf::solid_voxel(storage.block_from(record.payload), rank(record.mask, slot)),
    }
}
