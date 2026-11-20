//! The sparse 64-tree: the authoritative voxel world (decisions 0004 and
//! 0006).
//!
//! [`VoxelTree`] is the single writer. [`TreeSnapshot`] is an immutable,
//! thread-safe view, taken by copying one small handle per chunk; later writes
//! never change what it sees. Both read through [`TreeRead`], whose
//! [`NodeRef`] walk exposes the canonical structure to serialization, ray
//! traversal and the GPU encoder, and through the generic [`VoxelSource`]
//! interface they share with the [`DenseGrid`]s of dynamic objects.
//! [`TreeCursor`] makes runs of nearby lookups cheap.
//!
//! The world is stored as chunks: the subtree under each node at
//! [`CHUNK_LEVEL`] has its own arenas, so it can be built, loaded, compacted
//! and freed on its own, and a chunk-level region can be *not resident*: its
//! content is not loaded, reads say so, and edits there are refused.
//!
//! Modules: `node` is the storage layout (16-byte nodes, inline leaf records
//! and palette blocks), `alloc` the copy-on-write paged arenas, `forest` the
//! top store and the chunk arenas, `mutate` the write path that keeps the tree
//! canonical, `build` bulk building from region sources, `read` the read
//! interfaces and point lookups, `cursor` the cached lookups, `grid` the dense
//! grids.

mod alloc;
mod build;
mod chunks;
mod cursor;
mod forest;
mod grid;
mod mutate;
pub mod node;
mod read;

#[cfg(test)]
mod tests;

use std::fmt;

pub use alloc::{NODE_PAGE_LEN, WORD_PAGE_LEN};
pub use build::RegionSource;
pub use chunks::{ChunkData, ChunkInfo, MemoryBreakdown, representative};
pub use cursor::TreeCursor;
pub use grid::{DenseGrid, GridNode, MAX_GRID_LEVELS};
pub use node::{CHUNK_LEVEL, NOT_RESIDENT, Node, RECORD_WORDS, UNIFORM_BIT};
pub use read::{
    BranchRef, LeafRef, NodeKind, NodeRef, NodeView, Palette, TreeRead, VoxelSource, same_content,
};

use alloc::Storage;
use forest::{ChunkKeys, ChunkTable, Forest};
pub(crate) use mutate::Builder;
use mutate::{LocalBox, LocalEdit};
use node::{Child, path_key};
use read::TreeView;

use crate::consts::{TREE_DEPTH, WORLD_MIN_VOXEL};
use crate::coords::{VoxelBox, VoxelPos, path};
use crate::edit::{DirtyBricks, Edit, EditError};
use crate::material::MaterialId;

/// The level of the root node, whose children each cover a quarter of the
/// world per axis.
const ROOT_LEVEL: u32 = TREE_DEPTH - 1;

/// The lowest voxel of the world.
const WORLD_ORIGIN: VoxelPos = VoxelPos::new(WORLD_MIN_VOXEL, WORLD_MIN_VOXEL, WORLD_MIN_VOXEL);

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
    /// Node slots on the free lists.
    pub free_nodes: usize,
    /// Words in recycled blocks.
    pub free_words: usize,
    /// Bytes held by the writer's pages, live or free.
    pub bytes: usize,
    /// Live chunks: chunk-level regions with mixed content.
    pub chunks: usize,
}

/// The live, editable world tree. A new tree is all air.
///
/// Every write leaves the tree canonical, so equal content always has equal
/// structure and equal live [`stats`](Self::stats).
#[derive(Default)]
pub struct VoxelTree {
    forest: Forest,
    root: Child,
}

impl VoxelTree {
    /// An all-air tree. Allocates nothing until the first solid voxel.
    pub fn new() -> Self {
        Self::default()
    }

    /// A tree with nothing loaded: every region reads as not resident until
    /// it is built with [`build_region`](Self::build_region).
    pub fn unloaded() -> Self {
        Self {
            forest: Forest::default(),
            root: Child::NotResident,
        }
    }

    /// Sets one voxel. Returns whether its material changed. Fails, changing
    /// nothing, outside the world or in a region that is not resident.
    pub fn set(&mut self, pos: VoxelPos, material: MaterialId) -> Result<bool, EditError> {
        let local = path::world_local(pos).ok_or(EditError::OutOfWorld { pos })?;
        let mut changed = false;
        let edit = LocalEdit { local, material };
        // The write walks the voxel's path anyway: one that meets an unloaded
        // region is refused there, before anything changed.
        let refused = mutate::set_voxels(&mut self.forest, &mut self.root, &[edit], &mut |_, _| {
            changed = true;
        });
        if refused {
            return Err(EditError::NotResident { pos });
        }
        Ok(changed)
    }

    /// Sets every voxel of `region`. Returns whether anything changed. Fails,
    /// changing nothing, if any part of a non-empty region is outside the
    /// world or not resident.
    pub fn fill_box(&mut self, region: VoxelBox, material: MaterialId) -> Result<bool, EditError> {
        let Some(region) = LocalBox::of(region)? else {
            return Ok(false);
        };
        // A box inside a chunk the index knows is resident.
        let (lo, hi) = region.corners();
        let in_known_chunk = forest::chunk_key(lo) == forest::chunk_key(hi)
            && self.forest.indexed_chunk(forest::chunk_key(lo)).is_some();
        if !in_known_chunk && let Some(local) = self.view().unloaded_in(&region) {
            let pos = path::from_world_local(local);
            return Err(EditError::NotResident { pos });
        }
        Ok(mutate::fill_box(
            &mut self.forest,
            &mut self.root,
            &region,
            material,
        ))
    }

    /// Applies a batch in order: all of it, or none of it if any edit is
    /// outside the world or in a region that is not resident. Returns the
    /// bricks whose content changed.
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

        // Tree order keeps each chunk's edits together, so residency is
        // checked once per chunk.
        let view = self.view();
        let mut checked = None;
        for edit in &edits {
            let chunk = edit.local.map(|v| v >> chunk_shift());
            if checked != Some(chunk) {
                if !view.resident(edit.local) {
                    let pos = path::from_world_local(edit.local);
                    return Err(EditError::NotResident { pos });
                }
                checked = Some(chunk);
            }
        }

        // Each voxel is written once with its final material, so a change is
        // a net change of the batch.
        let mut bricks = Vec::new();
        let refused =
            mutate::set_voxels(&mut self.forest, &mut self.root, &edits, &mut |edit, _| {
                let brick = path::from_world_local(edit.local).brick();
                if bricks.last() != Some(&brick) {
                    bricks.push(brick);
                }
            });
        debug_assert!(!refused, "residency was checked above");
        Ok(DirtyBricks::from_unsorted(bricks))
    }

    /// A tree holding exactly what `source` describes for the whole world,
    /// built bottom-up (see [`RegionSource`]).
    pub fn from_source(source: &impl RegionSource) -> Self {
        let built: Result<Self, std::convert::Infallible> =
            Self::build_with(|builder| Ok(build::child(builder, source, ROOT_LEVEL, [0; 3])));
        match built {
            Ok(tree) => tree,
        }
    }

    /// Replaces the whole region of the node at `level` (1 to
    /// `TREE_DEPTH - 1`) whose lowest voxel is `min` with what `source`
    /// describes, built bottom-up and swapped in along one path. Building a
    /// chunk-level or larger region loads it if it was not resident.
    ///
    /// Returns whether the stored region was replaced: false only when it was
    /// and stays air or one uniform material. Fails, changing nothing, if
    /// `min` is outside the world, not the lowest voxel of such a node, or
    /// inside a smaller-than-chunk region whose chunk is not resident.
    pub fn build_region(
        &mut self,
        level: u32,
        min: VoxelPos,
        source: &impl RegionSource,
    ) -> Result<bool, EditError> {
        let local = node_origin(level, min)?;
        if level < CHUNK_LEVEL && !self.view().resident(local) {
            return Err(EditError::NotResident { pos: min });
        }
        Ok(mutate::graft(
            &mut self.forest,
            &mut self.root,
            ROOT_LEVEL,
            level,
            local,
            |builder| build::child(builder, source, level, local),
        ))
    }

    /// Unloads the region of the node at `level` ([`CHUNK_LEVEL`] or above)
    /// whose lowest voxel is `min`: it becomes not resident and its storage is
    /// freed. Returns whether anything changed.
    pub fn unload(&mut self, level: u32, min: VoxelPos) -> Result<bool, EditError> {
        let local = node_origin(level, min)?;
        if level < CHUNK_LEVEL {
            return Err(EditError::Misaligned { pos: min, level });
        }
        Ok(mutate::graft(
            &mut self.forest,
            &mut self.root,
            ROOT_LEVEL,
            level,
            local,
            |_| Child::NotResident,
        ))
    }

    /// Builds a tree bottom-up: `build` writes canonical content through the
    /// builder and returns the root child.
    pub(crate) fn build_with<E>(
        build: impl FnOnce(&mut Builder<'_>) -> Result<Child, E>,
    ) -> Result<Self, E> {
        let mut forest = Forest::default();
        let root = build(&mut Builder::new(&mut forest))?;
        Ok(Self { forest, root })
    }

    /// An immutable view of the tree as it is now. Copies one handle per
    /// chunk; no voxel data.
    pub fn snapshot(&self) -> TreeSnapshot {
        TreeSnapshot {
            top: self.forest.top.storage().clone(),
            chunks: self.forest.snapshot_chunks(),
            keys: self.forest.chunk_keys().clone(),
            root: self.root,
        }
    }

    /// Live and free storage. The live counts depend only on the content.
    pub fn stats(&self) -> TreeStats {
        self.forest.stats()
    }

    /// The generation of the chunk holding `pos`: raised by every change to
    /// it and unique across the tree. `None` when that chunk-level region is
    /// air, one material, not resident or outside the world.
    pub fn chunk_generation(&self, pos: VoxelPos) -> Option<u64> {
        let local = path::world_local(pos)?;
        let id = self.view().chunk_id(local)?;
        Some(self.forest.chunk(id).generation)
    }

    /// The whole world as one child, as [`TreeRead::root`].
    pub fn root(&self) -> NodeRef<'_> {
        self.view().root()
    }

    /// The material of one voxel; air outside the world and where the region
    /// is not resident (see [`lookup`](Self::lookup)). Reads node and word
    /// values straight from storage.
    pub fn get(&self, pos: VoxelPos) -> MaterialId {
        self.view().get(pos)
    }

    /// The material of one voxel, or `None` where its region is not
    /// resident. Air outside the world.
    pub fn lookup(&self, pos: VoxelPos) -> Option<MaterialId> {
        self.view().lookup(pos)
    }

    /// Whether the region holding `pos` is loaded. True outside the world,
    /// which is air.
    pub fn is_resident(&self, pos: VoxelPos) -> bool {
        path::world_local(pos).is_none_or(|local| self.view().resident(local))
    }

    /// Cached point lookups for runs of nearby voxels.
    pub fn cursor(&self) -> TreeCursor<'_> {
        TreeCursor::new(self.view())
    }

    fn view(&self) -> TreeView<'_> {
        TreeView {
            top: self.forest.top.storage(),
            chunks: self.forest.chunk_slice(),
            keys: self.forest.chunk_keys(),
            root: self.root,
        }
    }
}

/// Bits to shift a world-local coordinate by to get its chunk coordinate.
const fn chunk_shift() -> u32 {
    2 * (CHUNK_LEVEL + 1)
}

/// The world-local lowest voxel of the node at `level` whose lowest voxel is
/// `min`, checking that `min` is one.
fn node_origin(level: u32, min: VoxelPos) -> Result<[u32; 3], EditError> {
    let misaligned = EditError::Misaligned { pos: min, level };
    if !(1..=ROOT_LEVEL).contains(&level) {
        return Err(misaligned);
    }
    let local = path::world_local(min).ok_or(EditError::OutOfWorld { pos: min })?;
    let size = path::child_span(level + 1);
    if local.iter().any(|&v| v % size != 0) {
        return Err(misaligned);
    }
    Ok(local)
}

impl TreeRead for VoxelTree {
    fn root(&self) -> NodeRef<'_> {
        VoxelTree::root(self)
    }

    fn get(&self, pos: VoxelPos) -> MaterialId {
        VoxelTree::get(self, pos)
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
    top: Storage,
    chunks: ChunkTable,
    keys: ChunkKeys,
    root: Child,
}

impl TreeSnapshot {
    /// The whole world as one child, as [`TreeRead::root`].
    pub fn root(&self) -> NodeRef<'_> {
        self.view().root()
    }

    /// The material of one voxel; air outside the world and where the region
    /// is not resident. Reads node and word values straight from storage.
    pub fn get(&self, pos: VoxelPos) -> MaterialId {
        self.view().get(pos)
    }

    /// The material of one voxel, or `None` where its region is not
    /// resident. Air outside the world.
    pub fn lookup(&self, pos: VoxelPos) -> Option<MaterialId> {
        self.view().lookup(pos)
    }

    /// Whether the region holding `pos` is loaded. True outside the world.
    pub fn is_resident(&self, pos: VoxelPos) -> bool {
        path::world_local(pos).is_none_or(|local| self.view().resident(local))
    }

    /// Cached point lookups for runs of nearby voxels.
    pub fn cursor(&self) -> TreeCursor<'_> {
        TreeCursor::new(self.view())
    }

    fn view(&self) -> TreeView<'_> {
        TreeView {
            top: &self.top,
            chunks: self.chunks.as_slice(),
            keys: &self.keys,
            root: self.root,
        }
    }
}

impl TreeRead for TreeSnapshot {
    fn root(&self) -> NodeRef<'_> {
        TreeSnapshot::root(self)
    }

    fn get(&self, pos: VoxelPos) -> MaterialId {
        TreeSnapshot::get(self, pos)
    }
}

impl fmt::Debug for TreeSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeSnapshot")
            .field("root", &self.root)
            .finish_non_exhaustive()
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
const _: () = assert!(CHUNK_LEVEL >= 1 && CHUNK_LEVEL < ROOT_LEVEL);

// Snapshots are handed to render and streaming threads (decision 0003).
const _: () = {
    const fn send_and_sync<T: Send + Sync>() {}
    send_and_sync::<TreeSnapshot>();
};

#[cfg(test)]
impl VoxelTree {
    /// Whether `snapshot` still shares every page of this tree: nothing was
    /// written, added or freed into a new chunk since it was taken.
    fn shares_everything_with(&self, snapshot: &TreeSnapshot) -> bool {
        self.root == snapshot.root
            && self
                .forest
                .top
                .storage()
                .shares_everything_with(&snapshot.top)
            && self.forest.shares_chunks_with(&snapshot.chunks)
    }
}
