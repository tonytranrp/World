//! Coherent point lookups that reuse the path of the previous lookup.

use std::fmt;

use super::ROOT_LEVEL;
use super::alloc::StorageReader;
use super::node::{Child, slot_at};
use super::read::{Step, TreeView, record_voxel, step};
use crate::consts::TREE_DEPTH;
use crate::coords::{VoxelPos, path};
use crate::material::MaterialId;

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
    /// Remembers the last node and word page across lookups too.
    storage: StorageReader<'a>,
    root: Child,
    /// World-local coordinate of the last lookup.
    last: [u32; 3],
    /// `path[level]` is the node at that level on the path to `last`, for
    /// levels `bottom` up to the root; when `bottom` is 0, `path[0]` is the
    /// word offset of the last leaf's record.
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
            storage: view.storage.reader(),
            root: view.root,
            last: [0; 3],
            path,
            bottom: ROOT_LEVEL,
        }
    }

    /// The material of one voxel; air outside the world.
    pub fn get(&mut self, pos: VoxelPos) -> MaterialId {
        let Some(local) = path::world_local(pos) else {
            return MaterialId::AIR;
        };
        match self.root {
            Child::Air => return MaterialId::AIR,
            Child::Uniform(m) => return m,
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
        if level == 0 {
            return record_voxel(&mut self.storage, self.path[0], slot_at(local, 0));
        }
        loop {
            match step(&mut self.storage, self.path[level as usize], level, local) {
                Step::Filled(m) => {
                    self.bottom = level;
                    return m;
                }
                Step::Node(index) => {
                    level -= 1;
                    self.path[level as usize] = index;
                }
                Step::Record(at) => {
                    self.path[0] = at;
                    self.bottom = 0;
                    return record_voxel(&mut self.storage, at, slot_at(local, 0));
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
