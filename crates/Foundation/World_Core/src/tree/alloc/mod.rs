//! The tree's memory: a node arena and a word arena in copy-on-write pages
//! (decision 0004).
//!
//! [`Storage`] is the shared, readable part: the two paged arrays. Cloning it is
//! O(1), which is how a snapshot is taken. [`Store`] is the writer: storage
//! plus the free lists and live counts. Every write goes through
//! `Arc::make_mut`, so a page a snapshot still holds is copied before it
//! changes and the snapshot never sees the write. Freeing writes no page, so a
//! freed slot or block is invisible to readers until it is reused.
//!
//! Allocation depends only on the sequence of calls, never on addresses or
//! timing, so the same edits always give the same indices.

mod blocks;
mod nodes;
mod pages;

use blocks::WordBlocks;
use nodes::NodeSlots;
use pages::{PageReader, Pages};
use std::sync::Arc;

use super::TreeStats;
use super::node::{MAX_BLOCK_LEN, Node};

// Page sizes. Both arenas use 16 KiB pages (four OS pages).
//
// After each snapshot (taken once per frame, decision 0003), the first write
// to a page copies the whole page. One edit rewrites at most one node and one
// block per level plus one palette block, so it touches at most
// `2 * TREE_DEPTH` pages: at 16 KiB the worst case is 320 KiB copied, about
// 20 us, once per page per frame, and edits in one area share pages so the
// usual cost is a small fraction of that. The first write after a snapshot
// also copies the page directory and each directory chunk it touches (see
// `pages`), a few hundred `Arc`s per GiB of storage rather than one per page.
// Smaller pages would copy less per edit but make the directory and the
// per-page overhead grow; larger pages the reverse.

/// Nodes per node page: 1024 x 16 bytes = 16 KiB.
pub const NODE_PAGE_LEN: usize = 1024;

/// Words per word page: 4096 x 4 bytes = 16 KiB.
pub const WORD_PAGE_LEN: usize = 4096;

const _: () = assert!(NODE_PAGE_LEN * size_of::<Node>() == 16 * 1024);
const _: () = assert!(WORD_PAGE_LEN * size_of::<u32>() == 16 * 1024);
const _: () = assert!(WORD_PAGE_LEN >= MAX_BLOCK_LEN);

/// The readable part of the arenas: two paged arrays. Cloning is O(1) and the
/// clone never changes.
#[derive(Clone, Default)]
pub(crate) struct Storage {
    nodes: Pages<Node, NODE_PAGE_LEN>,
    words: Pages<u32, WORD_PAGE_LEN>,
}

impl Storage {
    #[inline]
    pub fn node(&self, index: u32) -> Node {
        self.nodes.get(index)
    }

    #[inline]
    pub fn word(&self, index: u32) -> u32 {
        self.words.get(index)
    }

    /// The `len` words of the block at `base`.
    #[inline]
    pub fn block(&self, base: u32, len: usize) -> &[u32] {
        self.words.slice(base, len)
    }

    /// Reads that remember the last node page and word page they touched.
    pub fn reader(&self) -> StorageReader<'_> {
        StorageReader {
            nodes: self.nodes.reader(),
            words: self.words.reader(),
        }
    }

    /// Whether both hold the very same page directories: nothing was written
    /// or added between them. While either is alive, a write through any
    /// other copy copies the directory first, so equal directories mean equal
    /// content.
    pub fn shares_everything_with(&self, other: &Self) -> bool {
        self.nodes.shares_list(&other.nodes) && self.words.shares_list(&other.words)
    }
}

/// Point reads from a [`Storage`] that remember the last node page and word
/// page, for lookups that walk down the tree.
#[derive(Clone)]
pub(crate) struct StorageReader<'a> {
    nodes: PageReader<'a, Node, NODE_PAGE_LEN>,
    words: PageReader<'a, u32, WORD_PAGE_LEN>,
}

impl<'a> StorageReader<'a> {
    #[inline(always)]
    pub fn node(&mut self, index: u32) -> Node {
        self.nodes.get(index)
    }

    #[inline(always)]
    pub fn word(&mut self, index: u32) -> u32 {
        self.words.get(index)
    }

    /// The `len` words of the block at `base`.
    #[inline(always)]
    pub fn block(&mut self, base: u32, len: usize) -> &'a [u32] {
        self.words.slice(base, len)
    }

    /// The words from `start` to the end of its page: a whole block whose
    /// length is not known until its first word is read.
    #[inline(always)]
    pub fn block_from(&mut self, start: u32) -> &'a [u32] {
        self.words.tail(start)
    }
}

/// The writer's arenas: storage plus free lists and live counts.
///
/// The free lists sit behind `Arc`s too, so the first write after a snapshot
/// copies them only if it allocates or frees: a write in place copies one
/// page and nothing else.
#[derive(Clone, Default)]
pub(crate) struct Store {
    storage: Storage,
    nodes: Arc<NodeSlots>,
    blocks: Arc<WordBlocks>,
    /// Live leaves: records that are neither air nor uniform.
    leaves: usize,
}

impl Store {
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    pub fn node(&self, index: u32) -> Node {
        self.storage.node(index)
    }

    pub fn word(&self, index: u32) -> u32 {
        self.storage.word(index)
    }

    pub fn block(&self, base: u32, len: usize) -> &[u32] {
        self.storage.block(base, len)
    }

    /// Stores a new node and returns its index.
    pub fn alloc_node(&mut self, node: Node) -> u32 {
        let index = Arc::make_mut(&mut self.nodes).alloc(&mut self.storage.nodes);
        self.set_node(index, node);
        index
    }

    /// Overwrites a live node.
    pub fn set_node(&mut self, index: u32, node: Node) {
        self.storage.nodes.slice_mut(index, 1)[0] = node;
    }

    pub fn free_node(&mut self, index: u32) {
        Arc::make_mut(&mut self.nodes).free(index);
    }

    /// Stores a new block of 1 to 192 words and returns its base.
    pub fn alloc_block(&mut self, words: &[u32]) -> u32 {
        let base = Arc::make_mut(&mut self.blocks).alloc(words.len(), &mut self.storage.words);
        self.write_block(base, words);
        base
    }

    /// Overwrites a live block of exactly `words.len()` words.
    pub fn write_block(&mut self, base: u32, words: &[u32]) {
        self.storage
            .words
            .slice_mut(base, words.len())
            .copy_from_slice(words);
    }

    /// Replaces the block of `old_len` words at `base` with `words` and
    /// returns the new block's base: in place when the block is the arena's
    /// last and the new length fits its page, otherwise a new block, the old
    /// one freed.
    pub fn realloc_block(&mut self, base: u32, old_len: usize, words: &[u32]) -> u32 {
        // One copy-on-write check of the allocator for the three steps.
        let blocks = Arc::make_mut(&mut self.blocks);
        let new = if blocks.resize_last::<WORD_PAGE_LEN>(base, old_len, words.len()) {
            base
        } else {
            let new = blocks.alloc(words.len(), &mut self.storage.words);
            blocks.free(base, old_len);
            new
        };
        self.write_block(new, words);
        new
    }

    pub fn free_block(&mut self, base: u32, len: usize) {
        Arc::make_mut(&mut self.blocks).free(base, len);
    }

    /// Counts a leaf in or out: `was` and `is` say whether the record that
    /// changed was and is a leaf.
    pub fn count_leaf(&mut self, was: bool, is: bool) {
        match (was, is) {
            (false, true) => self.leaves += 1,
            (true, false) => self.leaves -= 1,
            _ => {}
        }
    }

    /// Cuts each arena's last page to the items in use. A built chunk's
    /// arenas are written once and seldom grow again, so the rest of those
    /// pages would be held for nothing; a later allocation there grows the
    /// page back.
    pub fn trim(&mut self) {
        self.storage.nodes.trim(self.nodes.end());
        self.storage.words.trim(self.blocks.end());
    }

    pub fn stats(&self) -> TreeStats {
        TreeStats {
            branches: self.nodes.live(),
            leaves: self.leaves,
            words: self.blocks.live_words(),
            free_nodes: self.nodes.free_count(),
            free_words: self.blocks.free_words(),
            chunks: 0,
            bytes: self.storage.nodes.held_len() * size_of::<Node>()
                + self.storage.words.held_len() * size_of::<u32>(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(mask: u64, base: u32) -> Node {
        Node {
            mask,
            base,
            info: 0,
        }
    }

    #[test]
    fn nodes_and_blocks_read_back() {
        let mut store = Store::default();
        let base = store.alloc_block(&[1, 2, 3]);
        let index = store.alloc_node(node(0b111, base));
        assert_eq!(store.node(index), node(0b111, base));
        assert_eq!(store.block(base, 3), [1, 2, 3]);
        assert_eq!(store.word(base + 1), 2);
        let mut reader = store.storage().reader();
        assert_eq!(reader.node(index), node(0b111, base));
        assert_eq!(
            (reader.word(base), reader.block(base + 1, 2)),
            (1, &[2, 3][..])
        );
        assert_eq!(reader.block_from(base)[..3], [1, 2, 3]);
        assert_eq!(reader.block_from(base).len(), WORD_PAGE_LEN);
    }

    #[test]
    fn stats_count_live_and_free_storage() {
        let mut store = Store::default();
        let a = store.alloc_block(&[7; 10]);
        let first = store.alloc_node(node(1, a));
        let b = store.alloc_block(&[8; 2]);
        let second = store.alloc_node(node(3, b));
        store.count_leaf(false, true);
        store.count_leaf(true, true);
        let s = store.stats();
        assert_eq!((s.branches, s.leaves, s.words), (2, 1, 12));
        assert_eq!(s.bytes, 2 * 16 * 1024);

        store.free_block(a, 10);
        store.free_node(first);
        store.free_block(b, 2);
        store.free_node(second);
        store.count_leaf(true, false);
        let s = store.stats();
        assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
        assert_eq!((s.free_nodes, s.free_words), (2, 12));
    }

    #[test]
    fn a_clone_keeps_its_contents_while_the_store_is_written() {
        let mut store = Store::default();
        let base = store.alloc_block(&[1, 2]);
        let index = store.alloc_node(node(0b11, base));
        let snapshot = store.storage().clone();
        assert!(snapshot.shares_everything_with(store.storage()));

        store.write_block(base, &[5, 6]);
        store.set_node(index, node(0b101, base));
        assert_eq!(snapshot.block(base, 2), [1, 2]);
        assert_eq!(snapshot.node(index), node(0b11, base));
        assert_eq!(store.block(base, 2), [5, 6]);
        assert!(!snapshot.shares_everything_with(store.storage()));
    }

    #[test]
    fn freeing_writes_no_page() {
        let mut store = Store::default();
        let base = store.alloc_block(&[1]);
        let index = store.alloc_node(node(1, base));
        let snapshot = store.storage().clone();
        store.free_block(base, 1);
        store.free_node(index);
        assert!(snapshot.shares_everything_with(store.storage()));
    }
}
