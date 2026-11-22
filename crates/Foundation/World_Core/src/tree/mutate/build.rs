//! Bottom-up construction and grafting (decision 0006).
//!
//! The builder writes canonical content straight into a store, children
//! before parents (in a chunk it opened, bricks as they come and the upper
//! nodes breadth first when the chunk closes), and never reads back what it
//! wrote: every leaf is packed once, every node block is allocated once at
//! its final size. It is how
//! decoding, terrain generation and compaction make subtrees without paying
//! for one write per voxel. A graft then swaps a finished subtree into place
//! along one path, freeing the subtree it replaces.

use super::Writer;
use super::branch::{self, Children};
use super::brick::{self, Records};
use crate::consts::TREE_FANOUT;
use crate::tree::forest::{Forest, chunk_key};
use crate::tree::node::CHUNK_LEVEL;
use crate::tree::node::leaf::{self, Record, Voxels};
use crate::tree::node::{Child, Node, Packed, UNIFORM_BIT, slot_at};

/// Writes canonical subtrees into the forest, bottom-up.
///
/// Content below the chunk level goes into a chunk: a caller building a
/// chunk-level node from the top opens it with
/// [`begin_chunk`](Self::begin_chunk) before building its children and
/// closes it with [`end_chunk`](Self::end_chunk).
///
/// In a chunk it opened, the builder stores bricks (and their palette
/// blocks, just before each brick's block) as they come, but holds the
/// nodes of level 2 and up until the chunk closes and then stores them
/// breadth first from the chunk root, nodes and blocks each in one run
/// (decision 0027). A lookup's upper levels then share a node page and a
/// few word pages, instead of each level-2 node sitting after its bricks.
pub(crate) struct Builder<'a> {
    writer: Writer<'a>,
    upper: Upper,
}

/// The nodes of level 2 and up of the chunk being built, held until it
/// closes. A held node is named in its parent's words by [`HELD`] and its
/// position here.
#[derive(Default)]
struct Upper {
    /// Whether nodes are being held: only in a chunk the builder opened.
    active: bool,
    /// Mask, first word in `words` and word count of each held node.
    nodes: Vec<(u64, u32, u32)>,
    words: Vec<u32>,
}

/// Marks a child word that names a held node. Real node indices of a chunk
/// stay far below it (2^30 nodes would be 16 GiB of nodes).
const HELD: u32 = 1 << 30;

impl<'a> Builder<'a> {
    pub fn new(forest: &'a mut Forest) -> Self {
        Self {
            writer: Writer::new(forest),
            upper: Upper::default(),
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
        self.writer.write(None, 1, &brick::pack(records))
    }

    /// The child at `level` (2 and up) made of these 64 canonical children.
    /// Air, one uniform material or all not resident collapses.
    pub fn branch(&mut self, level: u32, children: &Children) -> Child {
        debug_assert!(level >= 2, "level-1 nodes are built from leaf records");
        let packed = branch::pack(children);
        if self.upper.active
            && level < CHUNK_LEVEL + 1
            && let Packed::Node(node) = &packed
        {
            let held = self.upper.nodes.len() as u32;
            let words = node.words();
            self.upper
                .nodes
                .push((node.mask, self.upper.words.len() as u32, words.len() as u32));
            self.upper.words.extend_from_slice(words);
            return Child::Node(HELD | held);
        }
        self.writer.write(None, level, &packed)
    }

    /// Stores the held nodes under `root`, breadth first from it, and
    /// returns the root as stored. Any other child is returned as it is.
    fn place_upper(&mut self, root: Child) -> Child {
        let upper = std::mem::take(&mut self.upper);
        let child = match root {
            Child::Node(index) if index & HELD != 0 => {
                // Breadth first: the root, its children, their children.
                let mut order = vec![index & !HELD];
                let mut next = 0;
                while let Some(&held) = order.get(next) {
                    let (_, start, len) = upper.nodes[held as usize];
                    let words = &upper.words[start as usize..(start + len) as usize];
                    order.extend(
                        words
                            .iter()
                            .filter(|&&w| w & UNIFORM_BIT == 0 && w & HELD != 0)
                            .map(|&w| w & !HELD),
                    );
                    next += 1;
                }
                debug_assert_eq!(order.len(), upper.nodes.len(), "every held node is used");
                // Nodes first, in one run, so their indices are known...
                let store = self.writer.store_mut();
                let mut placed = vec![0u32; upper.nodes.len()];
                for &held in &order {
                    placed[held as usize] = store.alloc_node(Node::default());
                }
                debug_assert!(placed.iter().all(|&i| i < HELD), "node indices reach HELD");
                // ...then the blocks, in the same order, with held names
                // replaced by those indices.
                let mut block = [0u32; TREE_FANOUT as usize];
                for &held in &order {
                    let (mask, start, len) = upper.nodes[held as usize];
                    let words = &upper.words[start as usize..(start + len) as usize];
                    for (out, &w) in block.iter_mut().zip(words) {
                        *out = if w & UNIFORM_BIT == 0 && w & HELD != 0 {
                            placed[(w & !HELD) as usize]
                        } else {
                            w
                        };
                    }
                    let base = store.alloc_block(&block[..len as usize]);
                    store.set_node(
                        placed[held as usize],
                        Node {
                            mask,
                            base,
                            info: 0,
                        },
                    );
                }
                Child::Node(placed[(index & !HELD) as usize])
            }
            other => other,
        };
        // Keep the buffers for the next chunk.
        self.upper = Upper {
            active: false,
            nodes: upper.nodes,
            words: upper.words,
        };
        self.upper.nodes.clear();
        self.upper.words.clear();
        child
    }

    /// Whether a chunk is open.
    pub fn in_chunk(&self) -> bool {
        self.writer.chunk.is_some()
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
        debug_assert!(self.upper.nodes.is_empty());
        self.upper.active = true;
    }

    /// Closes the open chunk, whose content is `root`, and returns the child
    /// its parent holds: the chunk id, or `root` itself when it is air, one
    /// material or not resident, in which case the chunk is dropped.
    /// [`end_chunk`](Self::end_chunk) for a chunk at world-local `origin`,
    /// which the chunk index learns.
    pub fn end_chunk_at(&mut self, root: Child, origin: [u32; 3]) -> Child {
        let child = self.end_chunk(root);
        if let Child::Node(id) = child {
            self.writer.forest.link(id, chunk_key(origin));
        }
        child
    }

    pub fn end_chunk(&mut self, root: Child) -> Child {
        debug_assert!(self.writer.chunk.is_some(), "end_chunk without begin_chunk");
        let root = self.place_upper(root);
        let Some(id) = self.writer.chunk.take() else {
            return root;
        };
        if let Some(chunk) = self.writer.held.take() {
            self.writer.forest.check_in(id, chunk);
        }
        match root {
            Child::Node(index) => {
                let chunk = self.writer.forest.chunk_mut(id);
                chunk.root = index;
                chunk.store.trim();
                self.writer.forest.touch(id);
                Child::Node(id)
            }
            other => {
                self.writer.forest.free_chunk(id);
                other
            }
        }
    }

    /// Takes a whole chunk built elsewhere and returns the child its parent
    /// holds. Only valid where a chunk-level node is being built from the top.
    pub fn adopt_chunk(&mut self, chunk: std::sync::Arc<crate::tree::forest::Chunk>) -> Child {
        debug_assert!(self.writer.chunk.is_none(), "chunks do not nest");
        Child::Node(self.writer.forest.adopt_chunk(chunk))
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
            // The builder writes the same chunk: it takes the checked-out
            // chunk along and hands it back.
            let mut builder = Builder {
                writer: Writer {
                    forest: &mut *self.forest,
                    chunk: self.chunk,
                    held: self.held.take(),
                    refused: false,
                },
                upper: Upper::default(),
            };
            let new = make(&mut builder);
            self.held = builder.writer.held.take();
            // A chunk placed at the chunk level from the top sits at `target`.
            if level == CHUNK_LEVEL
                && self.chunk.is_none()
                && let Child::Node(id) = new
            {
                self.forest.link(id, chunk_key(target));
            }
            // A freed node may come back with the same index, so only a
            // filled child that stays the same is "no change".
            let same = new == child && !matches!(child, Child::Node(_));
            return (!same).then_some(new);
        }
        if self.at_chunk_boundary(level) {
            return self.in_chunk(child, target, |w, local| {
                w.graft(local, level, target_level, target, make)
            });
        }
        let slot = slot_at(target, level) as usize;
        let existing = self.existing(child);
        let mut children = self.children(child);
        let replaced = self.graft(children[slot], level - 1, target_level, target, make)?;
        children[slot] = replaced;
        Some(self.write(existing, level, &branch::pack(&children)))
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
        // The open chunk is checked out of the forest: read its own store.
        let s = b.writer.store().stats();
        assert_eq!((s.branches, s.leaves), (1, 1));

        b.discard(child, 1);
        let s = b.writer.store().stats();
        assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
        assert_eq!(b.end_chunk(Child::Air), Child::Air);
        assert_eq!(forest.stats().chunks, 0);
    }
}
