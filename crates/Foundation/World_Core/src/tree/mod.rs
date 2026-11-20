//! The sparse 64-tree: the authoritative voxel world (decision 0004).
//!
//! [`VoxelTree`] is the single writer. [`TreeSnapshot`] is an immutable,
//! thread-safe view taken in O(1); later writes never change what it sees.
//! Both read through [`TreeRead`], whose [`NodeRef`] walk exposes the canonical
//! structure to serialization, ray traversal and the GPU mirror, and through
//! the generic [`VoxelSource`] interface they share with the [`DenseGrid`]s
//! of dynamic objects. [`TreeCursor`] makes runs of nearby lookups cheap.
//!
//! Modules: `node` is the storage layout (16-byte nodes, inline leaf records
//! and palette blocks), `alloc` the copy-on-write paged arenas, `mutate` the
//! write path that keeps the tree canonical, `read` the read interfaces and
//! point lookups, `cursor` the cached lookups, `grid` the dense grids.

mod alloc;
mod build;
mod cursor;
mod grid;
mod mutate;
pub mod node;
mod read;

#[cfg(test)]
mod tests;

use std::fmt;

pub use alloc::{NODE_PAGE_LEN, WORD_PAGE_LEN};
pub use build::RegionSource;
pub use cursor::TreeCursor;
pub use grid::{DenseGrid, GridNode, MAX_GRID_LEVELS};
pub use node::{Node, RECORD_WORDS, UNIFORM_BIT};
pub use read::{
    BranchRef, LeafRef, NodeKind, NodeRef, NodeView, Palette, TreeRead, VoxelSource, same_content,
};

use alloc::{Storage, Store};
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
    /// describes, built bottom-up and swapped in along one path. Returns
    /// whether the stored region was replaced: false only when it was and
    /// stays air or one uniform material. Fails, changing nothing, if `min` is
    /// outside the world or not the lowest voxel of such a node.
    pub fn build_region(
        &mut self,
        level: u32,
        min: VoxelPos,
        source: &impl RegionSource,
    ) -> Result<bool, EditError> {
        let local = node_origin(level, min)?;
        let mut builder = mutate::Builder::new(&mut self.store);
        let new = build::child(&mut builder, source, level, local);
        Ok(mutate::graft(
            &mut self.store,
            &mut self.root,
            ROOT_LEVEL,
            level,
            local,
            new,
        ))
    }

    /// Builds a tree bottom-up: `build` writes canonical content through the
    /// builder and returns the root child.
    pub(crate) fn build_with<E>(
        build: impl FnOnce(&mut Builder<'_>) -> Result<Child, E>,
    ) -> Result<Self, E> {
        let mut store = Store::default();
        let root = build(&mut Builder::new(&mut store))?;
        Ok(Self { store, root })
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

    /// The whole world as one child, as [`TreeRead::root`].
    pub fn root(&self) -> NodeRef<'_> {
        self.view().root()
    }

    /// The material of one voxel; air outside the world. Reads node and word
    /// values straight from storage.
    pub fn get(&self, pos: VoxelPos) -> MaterialId {
        self.view().get(pos)
    }

    /// Cached point lookups for runs of nearby voxels.
    pub fn cursor(&self) -> TreeCursor<'_> {
        TreeCursor::new(self.view())
    }

    fn view(&self) -> TreeView<'_> {
        TreeView {
            storage: self.store.storage(),
            root: self.root,
        }
    }
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

    /// The whole world as one child, as [`TreeRead::root`].
    pub fn root(&self) -> NodeRef<'_> {
        self.view().root()
    }

    /// The material of one voxel; air outside the world. Reads node and word
    /// values straight from storage.
    pub fn get(&self, pos: VoxelPos) -> MaterialId {
        self.view().get(pos)
    }

    /// Cached point lookups for runs of nearby voxels.
    pub fn cursor(&self) -> TreeCursor<'_> {
        TreeCursor::new(self.view())
    }

    fn view(&self) -> TreeView<'_> {
        TreeView {
            storage: &self.storage,
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

// Snapshots are handed to render and streaming threads (decision 0003).
const _: () = {
    const fn send_and_sync<T: Send + Sync>() {}
    send_and_sync::<TreeSnapshot>();
};
