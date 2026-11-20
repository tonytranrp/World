//! Bottom-up construction and grafting (decision 0006).
//!
//! The builder writes canonical content straight into a store, children
//! before parents, and never reads back what it wrote: every leaf is packed
//! once, every node block is allocated once at its final size. It is how
//! decoding, terrain generation and compaction make subtrees without paying
//! for one write per voxel. A graft then swaps a finished subtree into place
//! along one path, freeing the subtree it replaces.

use super::Writer;
use super::branch::{self, Children};
use super::brick::{self, Records};
use crate::tree::forest::Forest;
use crate::tree::node::leaf::{self, Record, Voxels};
use crate::tree::node::{Child, slot_at};

/// Writes canonical subtrees into the forest, bottom-up.
///
/// Content below the chunk level goes into a chunk: a caller building a
/// chunk-level node from the top opens it with
/// [`begin_chunk`](Self::begin_chunk) before building its children and
/// closes it with [`end_chunk`](Self::end_chunk).
pub(crate) struct Builder<'a> {
    writer: Writer<'a>,
}

impl<'a> Builder<'a> {
    pub fn new(forest: &'a mut Forest) -> Self {
        Self {
            writer: Writer::new(forest),
        }
    }

    /// Stores one leaf and returns its record: air, a single-material record,
    /// or a record with a freshly allocated palette block.
    pub fn leaf(&mut self, voxels: &Voxels) -> Record {
        self.writer.store_leaf(Record::AIR, leaf::encode(voxels))
    }

    /// The level-1 child made of these 64 leaves, which must be canonical
    /// records made by [`leaf`](Self::leaf) (or air and uniform records). Air
    /// or one uniform material collapses into a filled child.
    pub fn brick(&mut self, records: &Records) -> Child {
        self.writer.write(None, 1, brick::pack(records))
    }

    /// The child at `level` (2 and up) made of these 64 canonical children.
    /// Air, one uniform material or all not resident collapses.
    pub fn branch(&mut self, level: u32, children: &Children) -> Child {
        debug_assert!(level >= 2, "level-1 nodes are built from leaf records");
        self.writer.write(None, level, branch::pack(children))
    }

    /// Whether a node built at `level` now roots a new chunk.
    pub fn enters_chunk(&self, level: u32) -> bool {
        self.writer.at_chunk_boundary(level)
    }

    /// Opens a chunk: everything built until [`end_chunk`](Self::end_chunk)
    /// goes into it.
    pub fn begin_chunk(&mut self) {
        debug_assert!(self.writer.chunk.is_none(), "chunks do not nest");
        self.writer.chunk = Some(self.writer.forest.alloc_chunk());
    }

    /// Closes the open chunk, whose content is `root`, and returns the child
    /// its parent holds: the chunk id, or `root` itself when it is air, one
    /// material or not resident, in which case the chunk is dropped.
    pub fn end_chunk(&mut self, root: Child) -> Child {
        debug_assert!(self.writer.chunk.is_some(), "end_chunk without begin_chunk");
        let Some(id) = self.writer.chunk.take() else {
            return root;
        };
        match root {
            Child::Node(index) => {
                self.writer.forest.chunk_mut(id).root = index;
                self.writer.forest.touch(id);
                Child::Node(id)
            }
            other => {
                self.writer.forest.free_chunk(id);
                other
            }
        }
    }

    /// Frees a subtree made by this builder that will not be used.
    #[cfg(test)]
    pub fn discard(&mut self, child: Child, level: u32) {
        self.writer.free_subtree(child, level);
    }
}

/// Replaces the child at `target_level` whose region contains world-local
/// `target` with the subtree `make` builds. `root` is the child at
/// `root_level`. The replaced subtree is freed first, so the new one reuses
/// its space; each node on the path is repacked, collapsing any that become
/// uniform. Returns whether the tree changed.
pub(crate) fn graft(
    forest: &mut Forest,
    root: &mut Child,
    root_level: u32,
    target_level: u32,
    target: [u32; 3],
    make: impl FnOnce(&mut Builder<'_>) -> Child,
) -> bool {
    debug_assert!((1..=root_level).contains(&target_level));
    let changed = Writer::new(forest).graft(*root, root_level, target_level, target, make);
    changed.map(|child| *root = child).is_some()
}

impl Writer<'_> {
    fn graft(
        &mut self,
        child: Child,
        level: u32,
        target_level: u32,
        target: [u32; 3],
        make: impl FnOnce(&mut Builder<'_>) -> Child,
    ) -> Option<Child> {
        if level == target_level {
            self.free_subtree(child, level);
            let mut builder = Builder {
                writer: Writer {
                    forest: &mut *self.forest,
                    chunk: self.chunk,
                },
            };
            let new = make(&mut builder);
            // A freed node may come back with the same index, so only a
            // filled child that stays the same is "no change".
            let same = new == child && !matches!(child, Child::Node(_));
            return (!same).then_some(new);
        }
        if self.at_chunk_boundary(level) {
            return self.in_chunk(child, |w, local| {
                w.graft(local, level, target_level, target, make)
            });
        }
        let slot = slot_at(target, level) as usize;
        let existing = self.existing(child);
        let mut children = self.children(child);
        let replaced = self.graft(children[slot], level - 1, target_level, target, make)?;
        children[slot] = replaced;
        Some(self.write(existing, level, branch::pack(&children)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::MaterialId;
    use crate::tree::node::leaf::{PackedLeaf, Voxels};

    const fn filled_voxels(material: MaterialId) -> Voxels {
        [material; 64]
    }

    #[test]
    fn uniform_leaves_and_bricks_collapse_without_allocating() {
        let mut forest = Forest::default();
        let mut b = Builder::new(&mut forest);
        let stone = b.leaf(&filled_voxels(MaterialId(2)));
        assert_eq!(stone, Record::filled(MaterialId(2)));
        assert_eq!(b.brick(&[stone; 64]), Child::Uniform(MaterialId(2)));
        assert_eq!(b.brick(&[Record::AIR; 64]), Child::Air);
        let s = forest.stats();
        assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
    }

    #[test]
    fn mixed_leaves_allocate_palettes_and_count_as_leaves() {
        let mut forest = Forest::default();
        let mut b = Builder::new(&mut forest);
        b.begin_chunk();
        let mut voxels = filled_voxels(MaterialId(1));
        voxels[9] = MaterialId(4);
        let record = b.leaf(&voxels);
        assert!(matches!(leaf::encode(&voxels), PackedLeaf::Palette { .. }));
        let mut records = [Record::AIR; 64];
        records[0] = record;
        let child = b.brick(&records);
        assert!(matches!(child, Child::Node(_)));
        let s = b.writer.forest.stats();
        assert_eq!((s.branches, s.leaves, s.chunks), (1, 1, 1));

        b.discard(child, 1);
        let s = b.writer.forest.stats();
        assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
    }
}
