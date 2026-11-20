//! Whole-chunk operations (decision 0006): listing chunks, compacting them,
//! copying regions, coarse summaries for distant detail, edit overlays
//! against the generated terrain, and a byte breakdown of the storage.
//!
//! Everything here builds through the bottom-up builder, so a copy, a
//! compaction or a merge writes each node once into fresh arenas, depth
//! first: children end up next to their parents and the result has no holes.

use std::collections::BTreeMap;

use super::build::RegionSource;
use super::mutate::{self, Builder};
use super::node::leaf::{Record, Voxels};
use super::node::{CHUNK_LEVEL, Child, slot_at, slots};
use super::read::{NodeRef, NodeView};
use super::{ROOT_LEVEL, TreeStats, VoxelTree, node_origin};
use crate::consts::TREE_FANOUT;
use crate::coords::{VoxelPos, path};
use crate::edit::EditError;
use crate::material::MaterialId;

/// One live chunk: a chunk-level region with mixed content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkInfo {
    /// The chunk's lowest voxel.
    pub min: VoxelPos,
    /// Raised by every change; unique across the tree.
    pub generation: u64,
    /// The chunk's own storage.
    pub stats: TreeStats,
}

/// The content of one chunk-level region, detached from its tree: air, one
/// material, or a whole chunk's arenas. Moving it between trees costs O(1),
/// which is how chunks built on worker threads join the world.
#[derive(Clone)]
pub struct ChunkData {
    content: ChunkContent,
}

#[derive(Clone)]
enum ChunkContent {
    Filled(Child),
    Chunk(std::sync::Arc<super::forest::Chunk>),
}

impl ChunkData {
    /// Bytes held by the chunk's pages; 0 for air or one material.
    pub fn bytes(&self) -> usize {
        match &self.content {
            ChunkContent::Filled(_) => 0,
            ChunkContent::Chunk(c) => c.store.stats().bytes,
        }
    }

    /// The material of every voxel, when the region is air or one material.
    pub fn uniform(&self) -> Option<MaterialId> {
        match self.content {
            ChunkContent::Filled(Child::Uniform(m)) => Some(m),
            ChunkContent::Filled(_) => Some(MaterialId::AIR),
            ChunkContent::Chunk(_) => None,
        }
    }
}

impl std::fmt::Debug for ChunkData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.content {
            ChunkContent::Filled(c) => write!(f, "ChunkData({c:?})"),
            ChunkContent::Chunk(c) => {
                write!(f, "ChunkData(chunk, {} bytes)", c.store.stats().bytes)
            }
        }
    }
}

/// Where the bytes of a tree go, for budgets and for deciding format changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryBreakdown {
    /// Nodes of level 2 and up, 16 bytes each.
    pub upper_nodes: usize,
    /// Level-1 nodes (bricks), 16 bytes each plus a 12-byte header.
    pub bricks: usize,
    /// Full leaves of their brick's default material: no bytes of their own.
    pub default_leaves: usize,
    /// Child words of level-2-and-up nodes, 4 bytes each.
    pub child_words: usize,
    /// Leaf records that are full and one material but not their brick's
    /// default (12 bytes each).
    pub full_records: usize,
    /// Leaf records of one material with some air (12 bytes each).
    pub single_records: usize,
    /// Leaf records with a palette (12 bytes each, plus their block).
    pub palette_records: usize,
    /// Words of palette blocks, by index width 1, 2, 4 and 8 bits.
    pub palette_words: [usize; 4],
    /// Words on free lists and page tails.
    pub free_words: usize,
    /// Bytes held by every page, live or free.
    pub held_bytes: usize,
}

impl MemoryBreakdown {
    /// Bytes of live content.
    pub fn live_bytes(&self) -> usize {
        16 * (self.upper_nodes + self.bricks)
            + 12 * self.bricks
            + 4 * self.child_words
            + 12 * (self.full_records + self.single_records + self.palette_records)
            + 4 * self.palette_words.iter().sum::<usize>()
    }
}

impl VoxelTree {
    /// Every live chunk, in tree order.
    pub fn chunks(&self) -> Vec<ChunkInfo> {
        let mut out = Vec::new();
        if let Child::Node(index) = self.root {
            self.list_chunks(index, ROOT_LEVEL, [0; 3], &mut out);
        }
        out
    }

    fn list_chunks(&self, index: u32, level: u32, origin: [u32; 3], out: &mut Vec<ChunkInfo>) {
        let top = self.forest.top.storage();
        let node = top.node(index);
        let span = path::child_span(level);
        for (k, slot) in slots(node.mask).enumerate() {
            let offset = path::slot_offset(slot);
            let child_origin = [0, 1, 2].map(|a| origin[a] + offset[a] * span);
            let Child::Node(child) = Child::from_word(top.word(node.base + k as u32)) else {
                continue;
            };
            if level == CHUNK_LEVEL + 1 {
                let chunk = self.forest.chunk(child);
                out.push(ChunkInfo {
                    min: path::from_world_local(child_origin),
                    generation: chunk.generation,
                    stats: TreeStats {
                        chunks: 1,
                        ..chunk.store.stats()
                    },
                });
            } else {
                self.list_chunks(child, level - 1, child_origin, out);
            }
        }
    }

    /// Rebuilds the chunk at `min` depth first into fresh arenas: same
    /// content, no free words, children next to their parents. Returns
    /// whether there was a chunk to compact.
    pub fn compact_chunk(&mut self, min: VoxelPos) -> Result<bool, EditError> {
        let local = node_origin(CHUNK_LEVEL, min)?;
        if self.view().chunk_id(local).is_none() {
            return Ok(false);
        }
        // The snapshot keeps the old chunk alive while the new one is built.
        let old = self.snapshot();
        let node = node_at(old.root(), CHUNK_LEVEL, local);
        mutate::graft(
            &mut self.forest,
            &mut self.root,
            ROOT_LEVEL,
            CHUNK_LEVEL,
            local,
            |builder| copy(builder, node, CHUNK_LEVEL),
        );
        Ok(true)
    }

    /// A new tree holding only the region of the node at `level` whose lowest
    /// voxel is `min`, at the same place; air everywhere else.
    pub fn extract(&self, level: u32, min: VoxelPos) -> Result<VoxelTree, EditError> {
        self.extract_with(level, min, |builder, node| copy(builder, node, level))
    }

    /// Like [`extract`](Self::extract), with every cell of `4^cell_level`
    /// voxels (`cell_level` 1 to `level`) reduced to one material: the most
    /// common solid material in it, or air when it is empty. This is the
    /// coarse summary an edited chunk keeps for distant detail.
    pub fn extract_coarse(
        &self,
        level: u32,
        min: VoxelPos,
        cell_level: u32,
    ) -> Result<VoxelTree, EditError> {
        if !(1..=level).contains(&cell_level) {
            return Err(EditError::Misaligned {
                pos: min,
                level: cell_level,
            });
        }
        self.extract_with(level, min, |builder, node| {
            coarse(builder, node, level, cell_level - 1)
        })
    }

    fn extract_with(
        &self,
        level: u32,
        min: VoxelPos,
        make: impl FnOnce(&mut Builder<'_>, NodeRef<'_>) -> Child,
    ) -> Result<VoxelTree, EditError> {
        let local = node_origin(level, min)?;
        let node = node_at(self.root(), level, local);
        VoxelTree::build_with(|builder| {
            // A region inside a chunk is built into one; `wrap` closes it.
            if level < CHUNK_LEVEL {
                builder.begin_chunk();
            }
            let child = make(builder, node);
            Ok(wrap(builder, child, level, local))
        })
    }

    /// The edit overlay of the chunk at `min` against `base`, the content it
    /// was generated with: a tree holding, inside that chunk, the chunk's
    /// material wherever it differs from the base and [`MaterialId::KEEP`]
    /// wherever it does not. An unedited chunk's overlay is one uniform KEEP
    /// child. Fails if the chunk is not resident.
    pub fn overlay(&self, min: VoxelPos, base: &impl RegionSource) -> Result<VoxelTree, EditError> {
        let local = node_origin(CHUNK_LEVEL, min)?;
        if !self.view().resident(local) {
            return Err(EditError::NotResident { pos: min });
        }
        let node = node_at(self.root(), CHUNK_LEVEL, local);
        VoxelTree::build_with(|builder| {
            let child = diff(builder, node, CHUNK_LEVEL, local, base);
            Ok(wrap(builder, child, CHUNK_LEVEL, local))
        })
    }

    /// Rebuilds the chunk at `min` as `base` with `overlay` on top: the
    /// overlay's material wherever it is not [`MaterialId::KEEP`], the base's
    /// elsewhere. This loads the chunk if it was not resident. Returns
    /// whether the stored chunk was replaced.
    pub fn apply_overlay(
        &mut self,
        min: VoxelPos,
        base: &impl RegionSource,
        overlay: &VoxelTree,
    ) -> Result<bool, EditError> {
        let local = node_origin(CHUNK_LEVEL, min)?;
        let source = Overlaid {
            base,
            overlay: node_at(overlay.root(), CHUNK_LEVEL, local),
            origin: local,
        };
        self.build_region(CHUNK_LEVEL, min, &source)
    }

    /// The content of the chunk-level region at `min`, detached: the region
    /// becomes not resident here. O(1) apart from the path above it. Fails if
    /// the region is not resident.
    pub fn take_chunk(&mut self, min: VoxelPos) -> Result<ChunkData, EditError> {
        let local = node_origin(CHUNK_LEVEL, min)?;
        if !self.view().resident(local) {
            return Err(EditError::NotResident { pos: min });
        }
        let content = match self.view().chunk_id(local) {
            Some(id) => ChunkContent::Chunk(self.forest.chunk_arc(id)),
            None => ChunkContent::Filled(match node_at(self.root(), CHUNK_LEVEL, local) {
                NodeRef::Uniform(m) => Child::Uniform(m),
                _ => Child::Air,
            }),
        };
        self.unload(CHUNK_LEVEL, min)?;
        Ok(ChunkData { content })
    }

    /// The content of the chunk-level region at `min`, shared rather than
    /// detached: O(1), and later edits to this tree copy what they change,
    /// so the returned data never sees them. `None` if not resident.
    pub fn chunk_data(&self, min: VoxelPos) -> Option<ChunkData> {
        let local = node_origin(CHUNK_LEVEL, min).ok()?;
        if !self.view().resident(local) {
            return None;
        }
        let content = match self.view().chunk_id(local) {
            Some(id) => ChunkContent::Chunk(self.forest.chunk_arc(id)),
            None => ChunkContent::Filled(match node_at(self.root(), CHUNK_LEVEL, local) {
                NodeRef::Uniform(m) => Child::Uniform(m),
                _ => Child::Air,
            }),
        };
        Some(ChunkData { content })
    }

    /// Puts detached chunk content at the chunk-level region at `min`,
    /// replacing whatever was there (loading it if it was not resident).
    /// O(1) apart from the path above it. Returns whether anything changed.
    pub fn insert_chunk(&mut self, min: VoxelPos, data: ChunkData) -> Result<bool, EditError> {
        let local = node_origin(CHUNK_LEVEL, min)?;
        Ok(mutate::graft(
            &mut self.forest,
            &mut self.root,
            ROOT_LEVEL,
            CHUNK_LEVEL,
            local,
            |builder| match data.content {
                ChunkContent::Filled(child) => child,
                ChunkContent::Chunk(chunk) => builder.adopt_chunk(chunk),
            },
        ))
    }

    /// Where the bytes go, node kind by node kind.
    pub fn memory_breakdown(&self) -> MemoryBreakdown {
        let stats = self.stats();
        let mut out = MemoryBreakdown {
            free_words: stats.free_words,
            held_bytes: stats.bytes,
            ..MemoryBreakdown::default()
        };
        tally(self.root(), &mut out);
        out
    }
}

/// The child standing where a node of `level` holds world-local `local`,
/// found from `root`, the level `ROOT_LEVEL` child.
fn node_at(root: NodeRef<'_>, level: u32, local: [u32; 3]) -> NodeRef<'_> {
    let mut node = root;
    let mut at = ROOT_LEVEL;
    while at > level {
        node = match node {
            NodeRef::Branch(b) => b.child(slot_at(local, at)),
            other => return other,
        };
        at -= 1;
    }
    node
}

/// Puts `child`, the subtree of the node at `level` and world-local `local`,
/// into an otherwise empty tree: one branch per level above it. A chunk the
/// caller opened for a region below the chunk level is closed at the chunk
/// level.
fn wrap(builder: &mut Builder<'_>, child: Child, level: u32, local: [u32; 3]) -> Child {
    let mut child = child;
    for at in level + 1..=ROOT_LEVEL {
        let mut children = [Child::Air; TREE_FANOUT as usize];
        children[slot_at(local, at) as usize] = child;
        child = builder.branch(at, &children);
        if at == CHUNK_LEVEL && builder.in_chunk() {
            child = builder.end_chunk(child);
        }
    }
    child
}

/// Builds a copy of `node`, a child at `level`.
fn copy(builder: &mut Builder<'_>, node: NodeRef<'_>, level: u32) -> Child {
    let NodeRef::Branch(b) = node else {
        return filled(node);
    };
    let opened = builder.enters_chunk(level);
    if opened {
        builder.begin_chunk();
    }
    let child = if level == 1 {
        let mut records = [Record::AIR; TREE_FANOUT as usize];
        for (slot, leaf) in b.children() {
            records[slot as usize] = match leaf {
                NodeRef::Leaf(l) => builder.leaf(&std::array::from_fn(|i| l.voxel(i as u32))),
                other => match filled(other) {
                    Child::Uniform(m) => Record::filled(m),
                    _ => Record::AIR,
                },
            };
        }
        builder.brick(&records)
    } else {
        let mut children = [Child::Air; TREE_FANOUT as usize];
        for (slot, c) in b.children() {
            children[slot as usize] = copy(builder, c, level - 1);
        }
        builder.branch(level, &children)
    };
    if opened {
        builder.end_chunk(child)
    } else {
        child
    }
}

/// The child of an air, uniform or not-resident view.
fn filled(node: NodeRef<'_>) -> Child {
    match node {
        NodeRef::Uniform(m) => Child::Uniform(m),
        NodeRef::NotResident => Child::NotResident,
        _ => Child::Air,
    }
}

/// Builds `node`, a child at `level`, with every child at `cut` (and so
/// every cell of `4^(cut + 1)` voxels) reduced to its most common solid
/// material.
fn coarse(builder: &mut Builder<'_>, node: NodeRef<'_>, level: u32, cut: u32) -> Child {
    if level == cut {
        return match representative(node, level) {
            Some(m) => Child::Uniform(m),
            None => filled(node),
        };
    }
    let NodeRef::Branch(b) = node else {
        return filled(node);
    };
    let opened = builder.enters_chunk(level);
    if opened {
        builder.begin_chunk();
    }
    let child = if level == 1 {
        // `cut` is 0 here: each leaf becomes one material.
        let mut records = [Record::AIR; TREE_FANOUT as usize];
        for (slot, leaf) in b.children() {
            records[slot as usize] = match representative(leaf, 0) {
                Some(m) => Record::filled(m),
                None => Record::AIR,
            };
        }
        builder.brick(&records)
    } else {
        let mut children = [Child::Air; TREE_FANOUT as usize];
        for (slot, c) in b.children() {
            children[slot as usize] = coarse(builder, c, level - 1, cut);
        }
        builder.branch(level, &children)
    };
    if opened {
        builder.end_chunk(child)
    } else {
        child
    }
}

/// The most common solid material of `node`, a child at `level` (0 for a
/// leaf), counting voxels exactly; ties go to the lower id. `None` when it
/// is air or not resident.
pub fn representative(node: impl NodeView, level: u32) -> Option<MaterialId> {
    let mut counts = BTreeMap::new();
    count(node, level, &mut counts);
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(m, _)| m)
}

fn count(node: impl NodeView, level: u32, counts: &mut BTreeMap<MaterialId, u128>) {
    use super::read::NodeKind;
    match node.kind() {
        NodeKind::Air | NodeKind::NotResident => {}
        // A child at `level` covers 4^(level + 1) voxels per axis.
        NodeKind::Uniform(m) => *counts.entry(m).or_default() += 1u128 << (6 * (level + 1)),
        NodeKind::Leaf => {
            for slot in 0..TREE_FANOUT {
                let m = node.voxel(slot);
                if !m.is_air() {
                    *counts.entry(m).or_default() += 1;
                }
            }
        }
        NodeKind::Branch => {
            for (_, child) in node.children() {
                count(child, level.saturating_sub(1), counts);
            }
        }
    }
}

/// Builds the overlay of `node`, a child at `level` whose lowest voxel is
/// world-local `origin`, against `base`.
fn diff(
    builder: &mut Builder<'_>,
    node: NodeRef<'_>,
    level: u32,
    origin: [u32; 3],
    base: &impl RegionSource,
) -> Child {
    let size = path::child_span(level + 1);
    let base_uniform = base.uniform(path::from_world_local(origin), size);
    match node {
        NodeRef::Air if base_uniform == Some(MaterialId::AIR) => {
            return Child::Uniform(MaterialId::KEEP);
        }
        NodeRef::Air => return Child::Air,
        NodeRef::Uniform(m) if base_uniform == Some(m) => return Child::Uniform(MaterialId::KEEP),
        NodeRef::Uniform(m) => return Child::Uniform(m),
        NodeRef::NotResident | NodeRef::Leaf(_) => return filled(node),
        NodeRef::Branch(_) => {}
    }
    let NodeRef::Branch(b) = node else {
        return Child::Air;
    };
    let opened = builder.enters_chunk(level);
    if opened {
        builder.begin_chunk();
    }
    let span = path::child_span(level);
    let child_origin = |slot: u32| {
        let offset = path::slot_offset(slot);
        [0, 1, 2].map(|a| origin[a] + offset[a] * span)
    };
    let child = if level == 1 {
        let mut records = [Record::AIR; TREE_FANOUT as usize];
        let mut base_voxels: Voxels = [MaterialId::AIR; TREE_FANOUT as usize];
        for (slot, record) in (0..).zip(records.iter_mut()) {
            let leaf_min = path::from_world_local(child_origin(slot));
            match base.uniform(leaf_min, 4) {
                Some(m) => base_voxels = [m; TREE_FANOUT as usize],
                None => base.leaf(leaf_min, &mut base_voxels),
            }
            let merged = b.child(slot);
            let overlay: Voxels = std::array::from_fn(|i| {
                let v = merged.voxel(i as u32);
                if v == base_voxels[i] {
                    MaterialId::KEEP
                } else {
                    v
                }
            });
            *record = builder.leaf(&overlay);
        }
        builder.brick(&records)
    } else {
        let mut children = [Child::Air; TREE_FANOUT as usize];
        for (slot, c) in (0..).zip(children.iter_mut()) {
            *c = diff(builder, b.child(slot), level - 1, child_origin(slot), base);
        }
        builder.branch(level, &children)
    };
    if opened {
        builder.end_chunk(child)
    } else {
        child
    }
}

/// A base with an overlay on top, as a region source for rebuilding a chunk.
struct Overlaid<'a, B: RegionSource> {
    base: &'a B,
    /// The overlay's chunk-level child.
    overlay: NodeRef<'a>,
    /// The chunk's world-local lowest voxel.
    origin: [u32; 3],
}

impl<B: RegionSource> Overlaid<'_, B> {
    /// The overlay's child covering the aligned cube at world-local `local`
    /// of `size` voxels (a power of four, at most a chunk).
    fn overlay_at(&self, local: [u32; 3], size: u32) -> NodeRef<'_> {
        let target = size.trailing_zeros() / 2; // 4^target = size
        let mut node = self.overlay;
        let mut level = CHUNK_LEVEL + 1;
        while level > target {
            node = match node {
                NodeRef::Branch(b) => b.child(slot_at(local, level - 1)),
                other => return other,
            };
            level -= 1;
        }
        node
    }

    fn inside(&self, local: [u32; 3]) -> bool {
        let chunk = path::child_span(CHUNK_LEVEL + 1);
        (0..3).all(|a| local[a] >= self.origin[a] && local[a] < self.origin[a] + chunk)
    }
}

impl<B: RegionSource> RegionSource for Overlaid<'_, B> {
    fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
        let local = path::world_local(min)?;
        if !self.inside(local) {
            return self.base.uniform(min, size);
        }
        match self.overlay_at(local, size) {
            NodeRef::Uniform(MaterialId::KEEP) => self.base.uniform(min, size),
            NodeRef::Uniform(m) => Some(m),
            NodeRef::Air => Some(MaterialId::AIR),
            _ => None,
        }
    }

    fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        self.base.leaf(min, voxels);
        let Some(local) = path::world_local(min) else {
            return;
        };
        if !self.inside(local) {
            return;
        }
        let over = self.overlay_at(local, 4);
        for (slot, voxel) in (0..).zip(voxels.iter_mut()) {
            let v = over.voxel(slot);
            if v != MaterialId::KEEP {
                *voxel = v;
            }
        }
    }
}

/// Adds the storage of `node` and everything under it to `out`.
fn tally(node: NodeRef<'_>, out: &mut MemoryBreakdown) {
    let NodeRef::Branch(b) = node else {
        return;
    };
    if let Some((header, _)) = b.brick_layout() {
        out.bricks += 1;
        out.default_leaves += header.full.count_ones() as usize;
        for (slot, leaf) in b.children() {
            match leaf {
                NodeRef::Uniform(_) if header.full & (1 << slot) != 0 => {}
                NodeRef::Uniform(_) => out.full_records += 1,
                NodeRef::Leaf(l) => match l.index_bits() {
                    0 => out.single_records += 1,
                    bits => {
                        out.palette_records += 1;
                        let palette = l.palette().count();
                        let words = 1 + palette.div_ceil(2) + l.index_words().len();
                        let class = bits.trailing_zeros() as usize;
                        out.palette_words[class.min(3)] += words;
                    }
                },
                _ => {}
            }
        }
    } else {
        out.upper_nodes += 1;
        out.child_words += b.mask().count_ones() as usize;
        for (_, child) in b.children() {
            tally(child, out);
        }
    }
}
