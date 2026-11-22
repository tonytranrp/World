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
use super::chunks::ChunkData;
use super::forest::{ChunkKeys, Chunks, chunk_key, chunk_root};
use super::mutate::LocalBox;
use super::node::leaf::{self, Record};
use super::node::{
    BRICK_FULL_MASK, BRICK_HEADER_WORDS, BRICK_PAIR_MASK, BrickHeader, CHUNK_LEVEL, Child,
    NOT_RESIDENT, Node, PAIR_WORDS, RECORD_WORDS, UNIFORM_BIT, bit, rank, slot_at, slots,
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

    /// The content of the chunk-level region at `min`, shared, for sources
    /// that store chunks as detachable arenas (trees and snapshots), so a
    /// reader can tell with [`ChunkData::same_arenas`] whether it is content
    /// it has seen. `None` for other sources and where it is not resident.
    fn shared_chunk(&self, min: VoxelPos) -> Option<ChunkData> {
        let _ = min;
        None
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

    /// For a leaf of two or more materials stored packed: its palette block
    /// exactly as decisions 0005 and 0025 lay it out (a header `p | b << 8`
    /// from three materials on, the palette two ids per word, then the
    /// indices LSB-first; a two-material block starts at its palette word,
    /// whose high half is never zero), so a consumer can copy it instead of
    /// reading 64 voxels. `None` when the source does not store leaves that
    /// way.
    fn palette_block(&self) -> Option<&[u32]> {
        None
    }

    /// For a level-1 node (a brick): passes its solid voxels to `each` as
    /// (material, count) pairs, a material possibly more than once, read
    /// from how the source stores the brick instead of leaf by leaf, and
    /// returns true. Returns false, passing nothing, when the view is not a
    /// level-1 node or the source has no such shortcut (the default).
    fn brick_counts(&self, each: &mut impl FnMut(MaterialId, u32)) -> bool {
        let _ = each;
        false
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

    /// As [`VoxelSource::shared_chunk`].
    fn shared_chunk(&self, min: VoxelPos) -> Option<ChunkData> {
        let _ = min;
        None
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

    fn shared_chunk(&self, min: VoxelPos) -> Option<ChunkData> {
        TreeRead::shared_chunk(self, min)
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
    pub(crate) fn new(storage: &'a Storage, chunks: &'a Chunks, child: Child, level: u32) -> Self {
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
                    info: node.info,
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
                let block = storage.block(base, len);
                if leaf::is_pair(block[0]) {
                    LeafContent::Pair(block)
                } else {
                    LeafContent::Palette(block)
                }
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

    fn children(&self) -> impl Iterator<Item = (u32, Self)> {
        // A branch reads its children in order (a brick's header once);
        // anything else repeats itself as the default does.
        let (branch, other) = match self {
            Self::Branch(b) => (Some(b.children()), None),
            _ => {
                let node = *self;
                (
                    None,
                    Some(slots(self.mask()).map(move |slot| (slot, node.child(slot)))),
                )
            }
        };
        branch
            .into_iter()
            .flatten()
            .chain(other.into_iter().flatten())
    }

    fn palette_block(&self) -> Option<&[u32]> {
        match self {
            Self::Leaf(LeafRef {
                content: LeafContent::Pair(block) | LeafContent::Palette(block),
                ..
            }) => Some(block),
            _ => None,
        }
    }

    fn brick_counts(&self, each: &mut impl FnMut(MaterialId, u32)) -> bool {
        match self {
            Self::Branch(b) => b.brick_counts(each),
            _ => false,
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
    chunks: &'a Chunks,
    level: u32,
    mask: u64,
    /// The node's `info`: at level 1, the block length and flags.
    info: u32,
    /// One child word (level 2 and up) per non-air child, in slot order, or
    /// a brick header and stored leaves (level 1).
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
        if self.level == 1 {
            let header = BrickHeader::read(self.block, self.info);
            if header.pairs & bit(slot) != 0 {
                return self.mixed_leaf(header.record_offset(self.mask, slot));
            }
            return NodeRef::leaf(self.storage, header.leaf(self.block, self.mask, slot));
        }
        self.child_at(rank(self.mask, slot))
    }

    /// The leaf of two or more materials whose five words start at `at` in
    /// this brick's block. A two-material leaf's palette block is in the
    /// brick's own words, after the leaf mask (decision 0031); a wider one's
    /// is apart, its offset followed by two zero words (decision 0035).
    #[inline]
    fn mixed_leaf(&self, at: usize) -> NodeRef<'a> {
        let block = self.block;
        let mask = u64::from(block[at]) | u64::from(block[at + 1]) << 32;
        if block[at + 3] | block[at + 4] == 0 {
            return NodeRef::leaf(self.storage, Record::palette(mask, block[at + 2]));
        }
        let len = 1 + leaf::index_words(mask.count_ones() as usize, 1);
        NodeRef::Leaf(LeafRef {
            mask,
            content: LeafContent::Pair(&block[at + 2..at + 2 + len]),
        })
    }

    /// The children that are not air, in ascending slot order.
    pub fn children(&self) -> impl Iterator<Item = (u32, NodeRef<'a>)> + 'a + use<'a> {
        let branch = *self;
        // A brick's leaves are read in order, the header once and each
        // leaf's words where the previous one's end, without ranking.
        let header = if self.level == 1 {
            BrickHeader::read(self.block, self.info)
        } else {
            BrickHeader::default()
        };
        let mut at = header.leaves_start();
        // The halves end the block.
        let mut half = 2 * (self.block.len() - header.half_words());
        slots(self.mask).enumerate().map(move |(k, slot)| {
            let child = if branch.level == 1 {
                branch.stored_leaf(header, &mut at, &mut half, slot)
            } else {
                branch.child_at(k)
            };
            (slot, child)
        })
    }

    /// Leaf `slot` of a brick with `header`, whose stored words start at
    /// `at` and, for a full leaf of another material, whose half is `half`
    /// (counted in halves from the block start); each moves past what the
    /// leaf uses.
    #[inline]
    fn stored_leaf(
        &self,
        header: BrickHeader,
        at: &mut usize,
        half: &mut usize,
        slot: u32,
    ) -> NodeRef<'a> {
        let b = bit(slot);
        let block = self.block;
        let mask_at = |i: usize| u64::from(block[i]) | u64::from(block[i + 1]) << 32;
        if header.default_full() & b != 0 {
            return NodeRef::Uniform(header.default);
        }
        if header.full & b != 0 {
            let word = block[*half / 2] >> (16 * (*half % 2));
            *half += 1;
            return NodeRef::Uniform(MaterialId(word as u16));
        }
        if header.defaults & b != 0 {
            *at += 2;
            return NodeRef::Leaf(LeafRef {
                mask: mask_at(*at - 2),
                content: LeafContent::Single(header.default),
            });
        }
        if header.pairs & b != 0 {
            *at += PAIR_WORDS;
            return self.mixed_leaf(*at - PAIR_WORDS);
        }
        *at += RECORD_WORDS;
        NodeRef::leaf(self.storage, Record::read(&block[*at - RECORD_WORDS..*at]))
    }

    /// [`NodeView::brick_counts`] for a level-1 node, from its header and
    /// stored leaves (decisions 0021 and 0025) without a view per leaf: the
    /// full default leaves are one count, an other full leaf one word, a
    /// default surface leaf or a record of one material its mask's
    /// population, a packed leaf its palette indices. False above level 1.
    pub fn brick_counts(&self, each: &mut impl FnMut(MaterialId, u32)) -> bool {
        if self.level != 1 {
            return false;
        }
        let header = BrickHeader::read(self.block, self.info);
        let default_full = header.default_full();
        if default_full != 0 {
            each(header.default, TREE_FANOUT * default_full.count_ones());
        }
        let halves = self.block.len() - header.half_words();
        for (k, _) in slots(header.other_full()).enumerate() {
            let word = self.block[halves + k / 2];
            each(MaterialId((word >> (16 * (k % 2))) as u16), TREE_FANOUT);
        }
        let mut at = header.leaves_start();
        for slot in slots(self.mask & !header.full) {
            let b = bit(slot);
            if header.defaults & b != 0 {
                let mask = u64::from(self.block[at]) | u64::from(self.block[at + 1]) << 32;
                each(header.default, mask.count_ones());
                at += 2;
                continue;
            }
            let record = if header.pairs & b != 0 {
                let mask = u64::from(self.block[at]) | u64::from(self.block[at + 1]) << 32;
                at += PAIR_WORDS;
                if self.block[at - 2] | self.block[at - 1] != 0 {
                    // Two materials, inline.
                    leaf::count_solid(&self.block[at - 3..at], mask.count_ones(), each);
                    continue;
                }
                Record::palette(mask, self.block[at - 3])
            } else {
                at += RECORD_WORDS;
                Record::read(&self.block[at - RECORD_WORDS..at])
            };
            let solid = record.mask.count_ones();
            match record.palette_base() {
                Some(base) => {
                    let len = leaf::block_len(record.mask, self.storage.word(base));
                    leaf::count_solid(self.storage.block(base, len), solid, each);
                }
                None => {
                    if let Some(m) = record.material() {
                        each(m, solid);
                    }
                }
            }
        }
        debug_assert!(at <= self.block.len(), "stored leaves overrun the block");
        true
    }

    /// For a level-1 node: its header and its block length in words, for
    /// storage accounting.
    pub(crate) fn brick_layout(&self) -> Option<(BrickHeader, usize)> {
        (self.level == 1).then(|| (BrickHeader::read(self.block, self.info), self.block.len()))
    }

    /// The `k`-th non-air child of a node of level 2 or higher.
    fn child_at(&self, k: usize) -> NodeRef<'a> {
        let child = Child::from_word(self.block[k]);
        if self.level == CHUNK_LEVEL + 1
            && let Child::Node(id) = child
        {
            // The children of a level-5 node root chunks.
            let (storage, root) = chunk_root(self.chunks, id);
            return NodeRef::new(storage, self.chunks, Child::Node(root), CHUNK_LEVEL);
        }
        NodeRef::new(self.storage, self.chunks, child, self.level - 1)
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
    /// Two materials: a palette block without a header (decision 0025).
    Pair(&'a [u32]),
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
            LeafContent::Pair(block) => leaf::pair_voxel(block, rank(self.mask, slot)),
            LeafContent::Palette(block) => leaf::headed_voxel(block, rank(self.mask, slot)),
        }
    }

    /// The solid materials present, ascending by id, each once.
    pub fn palette(&self) -> impl Iterator<Item = MaterialId> + 'a + use<'a> {
        let content = self.content;
        let len = match content {
            LeafContent::Single(_) => 1,
            LeafContent::Pair(_) => 2,
            LeafContent::Palette(block) => leaf::palette_len(block[0]),
        };
        (0..len).map(move |i| match content {
            LeafContent::Single(m) => m,
            LeafContent::Pair(block) | LeafContent::Palette(block) => leaf::palette_entry(block, i),
        })
    }

    /// Bits per packed palette index: the smallest of 0, 1, 2, 4 or 8 that
    /// indexes the palette, so 0 when it has one material.
    pub fn index_bits(&self) -> u32 {
        match self.content {
            LeafContent::Single(_) => 0,
            LeafContent::Pair(_) => 1,
            LeafContent::Palette(block) => leaf::index_bits(block[0]),
        }
    }

    /// The packed palette indices exactly as stored: one per solid voxel in
    /// slot order, [`index_bits`](Self::index_bits) wide, LSB-first in whole
    /// words. Empty when the width is 0.
    pub fn index_words(&self) -> &'a [u32] {
        match self.content {
            LeafContent::Single(_) => &[],
            LeafContent::Pair(block) => &block[1..],
            LeafContent::Palette(block) => &block[leaf::index_start(block[0])..],
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
    pub chunks: &'a Chunks,
    pub keys: &'a ChunkKeys,
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
        // A chunk the index knows: straight in, skipping the top levels.
        let mut level = ROOT_LEVEL;
        if let Some(id) = self.indexed_chunk(local) {
            level = CHUNK_LEVEL;
            index = id;
        }
        // Above the chunk level, where words may be not resident.
        let mut storage = self.top.reader();
        while level > CHUNK_LEVEL {
            match step::<true, false>(&mut storage, index, level, local) {
                Step::Filled(m) => return Some(m),
                Step::NotResident => return None,
                Step::Node(child) => {
                    level -= 1;
                    index = child;
                }
                // Records hang below level 1 only.
                Step::Record(_) | Step::Pair(_) => {
                    unreachable!("no records above the chunk level")
                }
            }
        }
        // Inside the chunk `index` names: everything is resident. The
        // branch levels share a loop without the brick step, which runs once
        // after it: inside the loop, its code cost every level registers
        // (decision 0025).
        let (chunk, root) = chunk_root(self.chunks, index);
        let mut storage = chunk.reader();
        index = root;
        while level > 1 {
            match step::<false, false>(&mut storage, index, level, local) {
                Step::Filled(m) => return Some(m),
                Step::Node(child) => {
                    level -= 1;
                    index = child;
                }
                Step::NotResident | Step::Record(_) | Step::Pair(_) => {
                    unreachable!("chunks are resident, and records hang below level 1")
                }
            }
        }
        Some(brick_voxel(&mut storage, index, local))
    }

    /// The live chunk holding world-local `local`, when the chunk index
    /// knows it.
    #[inline(always)]
    fn indexed_chunk(self, local: [u32; 3]) -> Option<u32> {
        let id = self.keys.get(chunk_key(local))?;
        matches!(self.chunks.get(id as usize), Some(Some(_))).then_some(id)
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
            match step::<true, false>(&mut storage, index, level, local) {
                Step::Node(child) if level == CHUNK_LEVEL + 1 => return Some(child),
                Step::Node(child) => {
                    index = child;
                    level -= 1;
                }
                Step::Filled(_) | Step::NotResident | Step::Record(_) | Step::Pair(_) => {
                    return None;
                }
            }
        }
    }

    /// Whether the region holding world-local `local` is loaded. Walks the
    /// top levels only.
    pub fn resident(self, local: [u32; 3]) -> bool {
        if self.indexed_chunk(local).is_some() {
            return true;
        }
        let mut index = match self.root {
            Child::Node(index) => index,
            Child::NotResident => return false,
            Child::Air | Child::Uniform(_) => return true,
        };
        let mut storage = self.top.reader();
        let mut level = ROOT_LEVEL;
        while level > CHUNK_LEVEL {
            match step::<true, false>(&mut storage, index, level, local) {
                Step::NotResident => return false,
                Step::Node(child) => {
                    index = child;
                    level -= 1;
                }
                Step::Filled(_) | Step::Record(_) | Step::Pair(_) => return true,
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
    /// The word offset of the inline two-material leaf that holds the voxel.
    Pair(u32),
}

/// The child of node `index`, at `level`, that holds local coordinate `local`.
/// `UNLOADED` says the node may hold not-resident words (it is above the
/// chunk level); `BRICK` says it may be a level-1 node (it is inside a
/// chunk). Callers that know which side they are on leave the other test out.
/// Always inlined: as a call, the reader's cached page goes through memory on
/// every level, which Callgrind measured at 70% more instructions per lookup.
#[inline(always)]
pub(crate) fn step<const UNLOADED: bool, const BRICK: bool>(
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
    if BRICK && level == 1 {
        // Most bricks have no full mask and no pair mask: the default mask
        // is the full leaves of the default and every other leaf is a
        // record, one rank as before decision 0021.
        if node.info & (BRICK_FULL_MASK | BRICK_PAIR_MASK) == 0 {
            let head = storage.block(node.base, BRICK_HEADER_WORDS);
            let defaults = u64::from(head[0]) | u64::from(head[1]) << 32;
            if defaults & bit(slot) != 0 {
                return Step::Filled(MaterialId(head[2] as u16));
            }
            let at = BRICK_HEADER_WORDS + RECORD_WORDS * rank(node.mask & !defaults, slot);
            return Step::Record(node.base + at as u32);
        }
        // The next most common: mixed leaves and no full mask, so every
        // stored leaf is a record or a mixed leaf (decisions 0031, 0035).
        if node.info & BRICK_FULL_MASK == 0 {
            let head = storage.block(node.base, BRICK_HEADER_WORDS + 2);
            let defaults = u64::from(head[0]) | u64::from(head[1]) << 32;
            let b = bit(slot);
            if defaults & b != 0 {
                return Step::Filled(MaterialId(head[2] as u16));
            }
            let pairs = u64::from(head[3]) | u64::from(head[4]) << 32;
            let at = node.base
                + (BRICK_HEADER_WORDS
                    + 2
                    + RECORD_WORDS * rank(node.mask & !defaults, slot)
                    + 2 * rank(pairs, slot)) as u32;
            return if pairs & b != 0 {
                Step::Pair(at)
            } else {
                Step::Record(at)
            };
        }
        return brick_step(storage, node, slot, local);
    }
    let k = rank(node.mask, slot) as u32;
    let word = storage.word(node.base + k);
    // Most words name a node; the not-resident word is one of the uniform
    // ones, so the common path tests a single bit.
    if word & UNIFORM_BIT == 0 {
        Step::Node(word)
    } else if UNLOADED && word == NOT_RESIDENT {
        Step::NotResident
    } else {
        Step::Filled(MaterialId(word as u16))
    }
}

/// Words of a brick header with the full mask and no pair mask.
const BRICK_FULL_HEADER_WORDS: usize = BRICK_HEADER_WORDS + 2;

/// [`step`] in a brick with a full mask (decisions 0025, 0031, 0033 and
/// 0035): leaves of the default material (full, or partial with a leaf
/// mask), other full leaves (a half each, ending the block), mixed leaves
/// and records.
#[inline(always)]
fn brick_step(storage: &mut StorageReader<'_>, node: Node, slot: u32, local: [u32; 3]) -> Step {
    let base = node.base;
    // The full mask is always present here; a mixed mask may follow it.
    let head = storage.block(base, BRICK_FULL_HEADER_WORDS);
    let defaults = u64::from(head[0]) | u64::from(head[1]) << 32;
    let full = u64::from(head[3]) | u64::from(head[4]) << 32;
    let default = MaterialId(head[2] as u16);
    let b = bit(slot);
    if defaults & full & b != 0 {
        return Step::Filled(default);
    }
    let others = full & !defaults;
    if others & b != 0 {
        // A full leaf of another material: a half of the block's last words.
        let k = rank(others, slot);
        let halves = node.block_len(1) - (others.count_ones() as usize).div_ceil(2);
        let word = storage.word(base + (halves + k / 2) as u32);
        return Step::Filled(MaterialId((word >> (16 * (k % 2))) as u16));
    }
    let partial = node.mask & !full;
    let mut at = base
        + (BRICK_FULL_HEADER_WORDS + 2 * rank(partial, slot) + rank(partial & !defaults, slot))
            as u32;
    if node.info & BRICK_PAIR_MASK != 0 {
        let p = storage.block(base + BRICK_FULL_HEADER_WORDS as u32, 2);
        let pairs = u64::from(p[0]) | u64::from(p[1]) << 32;
        at += 2 + 2 * rank(pairs, slot) as u32;
        if pairs & b != 0 {
            return Step::Pair(at);
        }
    }
    if defaults & b == 0 {
        return Step::Record(at);
    }
    // A default partial leaf: its leaf mask alone.
    let words = storage.block(at, 2);
    let leaf_mask = u64::from(words[0]) | u64::from(words[1]) << 32;
    Step::Filled(if leaf_mask & bit(slot_at(local, 0)) != 0 {
        default
    } else {
        MaterialId::AIR
    })
}

/// Voxel `slot` of the mixed leaf at word `at`: inline two materials, or
/// (zero index words) a palette block apart at the offset in word 2.
#[inline(always)]
pub(crate) fn pair_voxel(storage: &mut StorageReader<'_>, at: u32, slot: u32) -> MaterialId {
    let words = storage.block(at, PAIR_WORDS);
    let mask = u64::from(words[0]) | u64::from(words[1]) << 32;
    if mask & bit(slot) == 0 {
        return MaterialId::AIR;
    }
    if words[3] | words[4] == 0 {
        return leaf::solid_voxel(storage.block_from(words[2]), rank(mask, slot));
    }
    leaf::pair_voxel(&words[2..], rank(mask, slot))
}

/// Voxel `slot` of the leaf whose record is at word `at`. A record is one
/// material and air (decision 0035): wider leaves are mixed leaves.
#[inline(always)]
pub(crate) fn record_voxel(storage: &mut StorageReader<'_>, at: u32, slot: u32) -> MaterialId {
    let words = storage.block(at, RECORD_WORDS);
    let mask = u64::from(words[0]) | u64::from(words[1]) << 32;
    debug_assert!(words[2] & UNIFORM_BIT != 0, "a record holds one material");
    if mask & bit(slot) == 0 {
        MaterialId::AIR
    } else {
        MaterialId(words[2] as u16)
    }
}

/// The voxel at world-local `local` in level-1 node `index`: [`step`] at
/// level 1 and the leaf read after it in one, every branch ending in the
/// material, so a point lookup never builds and matches a [`Step`] (which
/// cost CI's instruction gate several percent per lookup).
#[inline(always)]
pub(crate) fn brick_voxel(
    storage: &mut StorageReader<'_>,
    index: u32,
    local: [u32; 3],
) -> MaterialId {
    let node = storage.node(index);
    let slot = slot_at(local, 1);
    let b = bit(slot);
    if node.mask & b == 0 {
        return MaterialId::AIR;
    }
    if node.info & BRICK_FULL_MASK == 0 {
        // No full mask: a full leaf is of the default, every other leaf a
        // record or (with a mixed mask) a mixed leaf. The default costs what
        // it did before decision 0021; the mixed mask is read only past it.
        let head = storage.block(node.base, BRICK_HEADER_WORDS);
        let defaults = u64::from(head[0]) | u64::from(head[1]) << 32;
        if defaults & b != 0 {
            return MaterialId(head[2] as u16);
        }
        let records = RECORD_WORDS * rank(node.mask & !defaults, slot);
        if node.info & BRICK_PAIR_MASK == 0 {
            let at = node.base + (BRICK_HEADER_WORDS + records) as u32;
            return record_voxel(storage, at, slot_at(local, 0));
        }
        let p = storage.block(node.base + BRICK_HEADER_WORDS as u32, 2);
        let pairs = u64::from(p[0]) | u64::from(p[1]) << 32;
        let at = node.base + (BRICK_HEADER_WORDS + 2 + records + 2 * rank(pairs, slot)) as u32;
        return if pairs & b != 0 {
            pair_voxel(storage, at, slot_at(local, 0))
        } else {
            record_voxel(storage, at, slot_at(local, 0))
        };
    }
    // A full mask: `brick_step`'s cases, each ending in the material.
    let base = node.base;
    let head = storage.block(base, BRICK_FULL_HEADER_WORDS);
    let defaults = u64::from(head[0]) | u64::from(head[1]) << 32;
    let full = u64::from(head[3]) | u64::from(head[4]) << 32;
    let default = MaterialId(head[2] as u16);
    if defaults & full & b != 0 {
        return default;
    }
    let others = full & !defaults;
    if others & b != 0 {
        let k = rank(others, slot);
        let halves = node.block_len(1) - (others.count_ones() as usize).div_ceil(2);
        let word = storage.word(base + (halves + k / 2) as u32);
        return MaterialId((word >> (16 * (k % 2))) as u16);
    }
    let partial = node.mask & !full;
    let mut at = base
        + (BRICK_FULL_HEADER_WORDS + 2 * rank(partial, slot) + rank(partial & !defaults, slot))
            as u32;
    if node.info & BRICK_PAIR_MASK != 0 {
        let p = storage.block(base + BRICK_FULL_HEADER_WORDS as u32, 2);
        let pairs = u64::from(p[0]) | u64::from(p[1]) << 32;
        at += 2 + 2 * rank(pairs, slot) as u32;
        if pairs & b != 0 {
            return pair_voxel(storage, at, slot_at(local, 0));
        }
    }
    if defaults & b == 0 {
        return record_voxel(storage, at, slot_at(local, 0));
    }
    // A default partial leaf: its leaf mask alone.
    let words = storage.block(at, 2);
    let leaf_mask = u64::from(words[0]) | u64::from(words[1]) << 32;
    if leaf_mask & bit(slot_at(local, 0)) != 0 {
        default
    } else {
        MaterialId::AIR
    }
}
