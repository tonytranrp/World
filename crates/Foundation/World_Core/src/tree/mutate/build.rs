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
use crate::tree::alloc::Store;
use crate::tree::node::leaf::{self, Record, Voxels};
use crate::tree::node::{Child, slot_at};

/// Writes canonical subtrees into a store, bottom-up.
pub(crate) struct Builder<'a> {
    writer: Writer<'a>,
}

impl<'a> Builder<'a> {
    pub fn new(store: &'a mut Store) -> Self {
        Self {
            writer: Writer { store },
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
    /// Air or one uniform material collapses into a filled child.
    pub fn branch(&mut self, level: u32, children: &Children) -> Child {
        debug_assert!(level >= 2, "level-1 nodes are built from leaf records");
        self.writer.write(None, level, branch::pack(children))
    }

    /// Frees a subtree made by this builder that will not be used.
    #[cfg(test)]
    pub fn discard(&mut self, child: Child, level: u32) {
        self.writer.free_subtree(child, level);
    }
}

/// Replaces the child at `target_level` whose region contains world-local
/// `target` with `new`, a canonical subtree already in `store`. `root` is the
/// child at `root_level`. Frees the replaced subtree and repacks each node on
/// the path, collapsing any that become one material. Returns whether the
/// tree changed.
pub(crate) fn graft(
    store: &mut Store,
    root: &mut Child,
    root_level: u32,
    target_level: u32,
    target: [u32; 3],
    new: Child,
) -> bool {
    debug_assert!((1..=root_level).contains(&target_level));
    let changed = Writer { store }.graft(*root, root_level, target_level, target, new);
    changed.map(|child| *root = child).is_some()
}

impl Writer<'_> {
    fn graft(
        &mut self,
        child: Child,
        level: u32,
        target_level: u32,
        target: [u32; 3],
        new: Child,
    ) -> Option<Child> {
        if level == target_level {
            if child == new {
                return None;
            }
            self.free_subtree(child, level);
            return Some(new);
        }
        let slot = slot_at(target, level) as usize;
        let existing = self.existing(child);
        let mut children = self.children(child);
        let replaced = self.graft(children[slot], level - 1, target_level, target, new)?;
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
        let mut store = Store::default();
        let mut b = Builder::new(&mut store);
        let stone = b.leaf(&filled_voxels(MaterialId(2)));
        assert_eq!(stone, Record::filled(MaterialId(2)));
        assert_eq!(b.brick(&[stone; 64]), Child::Uniform(MaterialId(2)));
        assert_eq!(b.brick(&[Record::AIR; 64]), Child::Air);
        let s = store.stats();
        assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
    }

    #[test]
    fn mixed_leaves_allocate_palettes_and_count_as_leaves() {
        let mut store = Store::default();
        let mut b = Builder::new(&mut store);
        let mut voxels = filled_voxels(MaterialId(1));
        voxels[9] = MaterialId(4);
        let record = b.leaf(&voxels);
        assert!(matches!(leaf::encode(&voxels), PackedLeaf::Palette { .. }));
        let mut records = [Record::AIR; 64];
        records[0] = record;
        let child = b.brick(&records);
        assert!(matches!(child, Child::Node(_)));
        let s = store.stats();
        assert_eq!((s.branches, s.leaves), (1, 1));

        Builder::new(&mut store).discard(child, 1);
        let s = store.stats();
        assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
    }
}
