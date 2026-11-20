//! The tree's storage: a shared top and one set of arenas per chunk
//! (decision 0006).
//!
//! Nodes at [`CHUNK_LEVEL`](super::node::CHUNK_LEVEL) and below live in the
//! arenas of their chunk; the few nodes above live in the top store. A
//! chunk is created when an edit, a build or a decode first gives the region
//! under a chunk-level child mixed content, and dropped whole, with all its
//! pages, when that region becomes air, one material or not resident. So
//! freeing a chunk returns its memory at once, and fragmentation and
//! copy-on-write stay inside one chunk.
//!
//! The chunk table and every chunk sit behind `Arc`s, so a snapshot is one
//! reference count and allocates nothing. The first write to a chunk after a
//! snapshot copies the table (one `Arc` per chunk) and that chunk's free
//! lists; its pages are then copied one by one as they are written, as in
//! `alloc`.
//!
//! Chunk ids are recycled last-in, first-out, so the same operations always
//! give the same ids.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use super::TreeStats;
use super::alloc::{Storage, Store};
use super::node::CHUNK_LEVEL;

/// One chunk's arenas and the index of its root node in them.
#[derive(Clone)]
pub(crate) struct Chunk {
    pub store: Store,
    pub root: u32,
    /// Raised by every change to the chunk; unique across the tree, so a
    /// recycled chunk id never repeats an old generation.
    pub generation: u64,
}

/// The live chunks by id, as the tree and its snapshots share them.
pub(crate) type ChunkTable = Arc<Vec<Option<Arc<Chunk>>>>;

/// Hashes a chunk key (30 bits) with one multiplication.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn write_u32(&mut self, v: u32) {
        self.0 = u64::from(v).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// Chunk ids by chunk key, for lookups that go straight to a chunk.
pub(crate) type ChunkIndex = HashMap<u32, u32, BuildHasherDefault<KeyHasher>>;

/// A chunk id's key when it has none (not known, or the id is free).
pub(crate) const NO_KEY: u32 = u32::MAX;

/// The key of the chunk holding world-local `local`: its chunk coordinate,
/// ten bits per axis.
#[inline]
pub(crate) const fn chunk_key(local: [u32; 3]) -> u32 {
    let shift = 2 * (CHUNK_LEVEL + 1);
    (local[0] >> shift) | (local[1] >> shift) << 10 | (local[2] >> shift) << 20
}

const _: () = assert!(
    crate::consts::WORLD_SIZE_VOXELS >> (2 * (CHUNK_LEVEL + 1)) <= 1 << 10,
    "a chunk coordinate fits ten bits per axis"
);

/// Where each live chunk sits, for readers: chunk key to id, and id to key.
/// A hit counts only when the id is live and its key matches, so a stale or
/// missing entry costs a walk from the root and never a wrong answer. Every
/// live chunk is linked at exactly one place, and a chunk is only freed, never
/// moved, so a matching key names that place.
#[derive(Clone, Default)]
pub(crate) struct ChunkKeys {
    pub index: Arc<ChunkIndex>,
    pub keys: Arc<Vec<u32>>,
}

impl ChunkKeys {
    /// The chunk id at `key`, if the index knows it.
    #[inline]
    pub fn get(&self, key: u32) -> Option<u32> {
        let id = *self.index.get(&key)?;
        (self.keys.get(id as usize) == Some(&key)).then_some(id)
    }

    fn link(&mut self, id: u32, key: u32) {
        // Already linked there: touch nothing a snapshot may share.
        if self.keys.get(id as usize) == Some(&key) {
            return;
        }
        let keys = Arc::make_mut(&mut self.keys);
        if keys.len() <= id as usize {
            keys.resize(id as usize + 1, NO_KEY);
        }
        keys[id as usize] = key;
        Arc::make_mut(&mut self.index).insert(key, id);
    }

    fn unlink(&mut self, id: u32) {
        let Some(&key) = self.keys.get(id as usize) else {
            return;
        };
        if key == NO_KEY {
            return;
        }
        Arc::make_mut(&mut self.keys)[id as usize] = NO_KEY;
        let index = Arc::make_mut(&mut self.index);
        if index.get(&key) == Some(&id) {
            index.remove(&key);
        }
    }
}

/// The writer's storage: the top store and the live chunks.
#[derive(Default)]
pub(crate) struct Forest {
    pub top: Store,
    chunks: ChunkTable,
    free: Vec<u32>,
    generation: u64,
    keys: ChunkKeys,
}

impl Forest {
    /// Opens a new, empty chunk and returns its id.
    pub fn alloc_chunk(&mut self) -> u32 {
        let chunk = Some(Arc::new(Chunk {
            store: Store::default(),
            root: 0,
            generation: 0,
        }));
        let chunks = Arc::make_mut(&mut self.chunks);
        match self.free.pop() {
            Some(id) => {
                chunks[id as usize] = chunk;
                id
            }
            None => {
                chunks.push(chunk);
                (chunks.len() - 1) as u32
            }
        }
    }

    /// Takes a chunk built elsewhere, as it is, and returns its new id.
    pub fn adopt_chunk(&mut self, chunk: Arc<Chunk>) -> u32 {
        let chunks = Arc::make_mut(&mut self.chunks);
        let id = match self.free.pop() {
            Some(id) => {
                chunks[id as usize] = Some(chunk);
                id
            }
            None => {
                chunks.push(Some(chunk));
                (chunks.len() - 1) as u32
            }
        };
        self.touch(id);
        id
    }

    /// The shared handle of a live chunk.
    pub fn chunk_arc(&self, id: u32) -> Arc<Chunk> {
        Arc::clone(
            self.chunks[id as usize]
                .as_ref()
                .expect("stored chunk ids are live"),
        )
    }

    /// Drops a chunk and everything in it.
    pub fn free_chunk(&mut self, id: u32) {
        Arc::make_mut(&mut self.chunks)[id as usize] = None;
        self.keys.unlink(id);
        self.free.push(id);
    }

    /// Records that chunk `id` now sits at chunk key `key` in the tree.
    pub fn link(&mut self, id: u32, key: u32) {
        self.keys.link(id, key);
    }

    /// The chunk index, for readers and snapshots.
    pub fn chunk_keys(&self) -> &ChunkKeys {
        &self.keys
    }

    /// The live chunk at chunk key `key`, when the index knows it.
    #[inline]
    pub fn indexed_chunk(&self, key: u32) -> Option<u32> {
        let id = self.keys.get(key)?;
        matches!(self.chunks.get(id as usize), Some(Some(_))).then_some(id)
    }

    /// A live chunk. Panics on a free id, which no stored word holds.
    #[inline(always)]
    pub fn chunk(&self, id: u32) -> &Chunk {
        self.chunks[id as usize]
            .as_ref()
            .expect("stored chunk ids are live")
    }

    /// A live chunk for writing; copies what a snapshot still shares.
    pub fn chunk_mut(&mut self, id: u32) -> &mut Chunk {
        let chunk = Arc::make_mut(&mut self.chunks)[id as usize]
            .as_mut()
            .expect("stored chunk ids are live");
        Arc::make_mut(chunk)
    }

    /// Takes chunk `id` out of the table for a run of writes, made unique
    /// (copied if a snapshot still shares it); [`check_in`](Self::check_in)
    /// puts it back. Until then the id reads as free, so only the writer that
    /// holds the chunk may touch it.
    pub fn check_out(&mut self, id: u32) -> Arc<Chunk> {
        let mut arc = Arc::make_mut(&mut self.chunks)[id as usize]
            .take()
            .expect("stored chunk ids are live");
        Arc::make_mut(&mut arc);
        arc
    }

    /// Puts back a chunk taken with [`check_out`](Self::check_out).
    pub fn check_in(&mut self, id: u32, chunk: Arc<Chunk>) {
        Arc::make_mut(&mut self.chunks)[id as usize] = Some(chunk);
    }

    /// Records a change to a chunk.
    pub fn touch(&mut self, id: u32) {
        let generation = self.next_generation();
        self.chunk_mut(id).generation = generation;
    }

    /// A generation no chunk has had.
    pub fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// Live and free storage of the top and every chunk.
    pub fn stats(&self) -> TreeStats {
        let mut total = self.top.stats();
        for chunk in self.chunks.iter().flatten() {
            let s = chunk.store.stats();
            total.branches += s.branches;
            total.leaves += s.leaves;
            total.words += s.words;
            total.free_nodes += s.free_nodes;
            total.free_words += s.free_words;
            total.bytes += s.bytes;
            total.chunks += 1;
        }
        total
    }

    /// The chunk table for a snapshot. O(1), allocates nothing.
    pub fn snapshot_chunks(&self) -> ChunkTable {
        Arc::clone(&self.chunks)
    }
}

/// The live chunks by id, as readers see them: the forest's table or a
/// snapshot's, which are the same type.
pub(crate) type Chunks = [Option<Arc<Chunk>>];

/// A chunk's storage and root, from the chunk id in a child word.
#[inline]
pub(crate) fn chunk_root(chunks: &Chunks, id: u32) -> (&Storage, u32) {
    let chunk = chunks[id as usize]
        .as_ref()
        .expect("stored chunk ids are live");
    (chunk.store.storage(), chunk.root)
}

impl Forest {
    /// The live chunks, for readers.
    pub fn chunk_slice(&self) -> &Chunks {
        &self.chunks
    }
}

#[cfg(test)]
impl Forest {
    /// Whether a snapshot's chunks are exactly this forest's, page for page.
    pub fn shares_chunks_with(&self, chunks: &ChunkTable) -> bool {
        Arc::ptr_eq(&self.chunks, chunks)
            || self.chunks.len() == chunks.len()
                && self
                    .chunks
                    .iter()
                    .zip(chunks.iter())
                    .all(|(a, b)| match (a, b) {
                        (None, None) => true,
                        (Some(a), Some(b)) => {
                            a.root == b.root
                                && a.store.storage().shares_everything_with(b.store.storage())
                        }
                        _ => false,
                    })
    }
}
