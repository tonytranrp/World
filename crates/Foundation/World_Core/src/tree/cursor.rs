//! Coherent point lookups that reuse the path of the previous lookup.

use std::fmt;

use super::ROOT_LEVEL;
use super::alloc::StorageReader;
use super::forest::{Chunks, chunk_root};
use super::node::{CHUNK_LEVEL, Child, slot_at};
use super::read::{Step, TreeView, pair_voxel, record_voxel, step};
use crate::consts::TREE_DEPTH;
use crate::coords::{VoxelPos, path};
use crate::material::MaterialId;

/// Set in a cached leaf offset (`path[0]`) when the leaf is an inline
/// two-material leaf. Word offsets of a chunk stay far below it.
const PAIR_AT: u32 = 1 << 31;

/// Point lookups on a tree or snapshot that remember the path to the last
/// voxel, like OpenVDB's value accessor.
///
/// A lookup starts from the lowest cached node that also contains the new
/// voxel instead of from the root, so walks along rays, neighbour reads and
/// scans of a region cost a level or two each rather than ten. Results are
/// exactly those of [`TreeRead::get`](super::TreeRead::get). A cursor borrows
/// its tree, so the tree cannot change while the cursor lives.
#[derive(Clone)]
pub struct TreeCursor<'a> {
    /// Reads the top levels; remembers the last node and word page.
    top: StorageReader<'a>,
    /// Reads the chunk on the cached path, when the path reaches one.
    chunk: Option<StorageReader<'a>>,
    chunks: &'a Chunks,
    root: Child,
    /// World-local coordinate of the last lookup.
    last: [u32; 3],
    /// `path[level]` is the node at that level on the path to `last`, for
    /// levels `bottom` up to the root; when `bottom` is 0, `path[0]` is the
    /// word offset of the last leaf's record, with [`PAIR_AT`] for an inline
    /// two-material leaf. Levels up to the chunk level
    /// index the chunk's arenas.
    path: [u32; TREE_DEPTH as usize],
    /// The lowest level whose `path` entry is valid.
    bottom: u32,
}

impl<'a> TreeCursor<'a> {
    pub(super) fn new(view: TreeView<'a>) -> Self {
        let mut path = [0; TREE_DEPTH as usize];
        if let Child::Node(index) = view.root {
            path[ROOT_LEVEL as usize] = index;
        }
        Self {
            top: view.top.reader(),
            chunk: None,
            chunks: view.chunks,
            root: view.root,
            last: [0; 3],
            path,
            bottom: ROOT_LEVEL,
        }
    }

    /// The material of one voxel; air outside the world and where the region
    /// is not resident.
    pub fn get(&mut self, pos: VoxelPos) -> MaterialId {
        self.lookup(pos).unwrap_or(MaterialId::AIR)
    }

    /// The material of one voxel, or `None` where its region is not
    /// resident. Air outside the world.
    pub fn lookup(&mut self, pos: VoxelPos) -> Option<MaterialId> {
        let Some(local) = path::world_local(pos) else {
            return Some(MaterialId::AIR);
        };
        match self.root {
            Child::Air => return Some(MaterialId::AIR),
            Child::Uniform(m) => return Some(m),
            Child::NotResident => return None,
            Child::Node(_) => {}
        }
        // A level-L node covers 4^(L+1) voxels per axis, so the old and new
        // paths share the node at every level where no axis differs at bit
        // 2L + 2 or above.
        let diff =
            (local[0] ^ self.last[0]) | (local[1] ^ self.last[1]) | (local[2] ^ self.last[2]);
        let shared = (u32::BITS - diff.leading_zeros()).saturating_sub(1) / 2;
        self.last = local;
        let mut level = shared.max(self.bottom);
        loop {
            let reader = if level > CHUNK_LEVEL {
                &mut self.top
            } else {
                // The cached path reaches this level, so it entered a chunk.
                self.chunk
                    .as_mut()
                    .expect("a cached path below the chunk level has a chunk")
            };
            if level == 0 {
                let at = self.path[0];
                return Some(if at & PAIR_AT != 0 {
                    pair_voxel(reader, at & !PAIR_AT, slot_at(local, 0))
                } else {
                    record_voxel(reader, at, slot_at(local, 0))
                });
            }
            // One inlined copy of the step serves both sides of the chunk
            // level; two cost more than the check they save.
            match step::<true, true>(reader, self.path[level as usize], level, local) {
                Step::Filled(m) => {
                    self.bottom = level;
                    return Some(m);
                }
                Step::NotResident => {
                    self.bottom = level;
                    return None;
                }
                Step::Node(child) => {
                    level -= 1;
                    self.path[level as usize] = child;
                    if level == CHUNK_LEVEL {
                        let (storage, root) = chunk_root(self.chunks, child);
                        self.chunk = Some(storage.reader());
                        self.path[level as usize] = root;
                    }
                }
                Step::Record(at) => {
                    self.path[0] = at;
                    self.bottom = 0;
                    level = 0;
                }
                Step::Pair(at) => {
                    self.path[0] = at | PAIR_AT;
                    self.bottom = 0;
                    level = 0;
                }
            }
        }
    }
}

impl fmt::Debug for TreeCursor<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeCursor")
            .field("root", &self.root)
            .field("last", &path::from_world_local(self.last))
            .field("bottom", &self.bottom)
            .finish_non_exhaustive()
    }
}
