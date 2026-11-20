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

use std::sync::Arc;

use super::TreeStats;
use super::alloc::{Storage, Store};

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

/// The writer's storage: the top store and the live chunks.
#[derive(Default)]
pub(crate) struct Forest {
    pub top: Store,
    chunks: ChunkTable,
    free: Vec<u32>,
    generation: u64,
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
        self.free.push(id);
    }

    /// A live chunk. Panics on a free id, which no stored word holds.
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

    /// Records a change to a chunk.
    pub fn touch(&mut self, id: u32) {
        self.generation += 1;
        let generation = self.generation;
        self.chunk_mut(id).generation = generation;
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

/// Finding a chunk's storage and root from the chunk id in a child word.
pub(crate) trait ChunkLookup: Sync {
    fn chunk_root(&self, id: u32) -> (&Storage, u32);
}

impl ChunkLookup for Forest {
    fn chunk_root(&self, id: u32) -> (&Storage, u32) {
        let chunk = self.chunk(id);
        (chunk.store.storage(), chunk.root)
    }
}

impl ChunkLookup for ChunkTable {
    fn chunk_root(&self, id: u32) -> (&Storage, u32) {
        let chunk = self[id as usize]
            .as_ref()
            .expect("stored chunk ids are live");
        (chunk.store.storage(), chunk.root)
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
