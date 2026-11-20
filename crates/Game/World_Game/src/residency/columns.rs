//! Column footprints shared by the residency workers.
//!
//! A chunk's columns (its 1024 x 1024 heightfield) depend only on the
//! generator and the chunk's `x` and `z`, yet they are most of the cost of
//! generating a chunk. Chunks stacked in one column share them through this
//! cache: the first load computes them, the others copy them. Two workers
//! asking for the same footprint at once compute it once (the second waits).
//!
//! The cache holds at most `cap` bytes of columns, least recently used
//! leaving first. Each footprint it has seen also leaves its
//! [`FootprintBounds`] (24 bytes) in a much longer list, so chunks that are
//! provably air or bedrock are decided without columns even after those
//! leave. Both lists live in ordered maps and evict by a use counter, so the
//! same sequence of requests always evicts the same entries (AGENTS.md
//! rule 11). Columns are a pure function of their key, so nothing here can
//! change what a chunk holds.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use world_gen::{ChunkColumns, ChunkCoord, FootprintBounds, Generator, GeneratorId};

/// Bounds kept at most: about 100 KiB, enough for every footprint of a
/// coarse disc several kilometres across.
const MAX_BOUNDS: usize = 4096;

/// Bytes of one full-resolution footprint: 1024 x 1024 columns of 8 bytes.
pub const FOOTPRINT_BYTES: usize = (world_gen::CHUNK_SIZE as usize).pow(2) * 8;

/// Which footprint: the generator (seed, version, parameters) and the
/// chunk's `x` and `z`.
type Key = (GeneratorId, u32, u32);

fn key(generator: &Generator, chunk: ChunkCoord) -> Key {
    (generator.id(), chunk.x(), chunk.z())
}

/// Columns being computed or ready. The slot is shared, so a worker waiting
/// on it, or holding the columns, is unaffected when the entry leaves.
type Slot = Arc<OnceLock<Arc<ChunkColumns>>>;

/// How often the cache answered, for [`super::world::ResidencyStats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheCounts {
    /// Footprints served from the cache (or from a worker computing them).
    pub hits: u64,
    /// Footprints computed.
    pub misses: u64,
}

#[derive(Default)]
struct Inner {
    /// Raised by every use; the smallest value leaves first.
    clock: u64,
    columns: BTreeMap<Key, (Slot, u64)>,
    bounds: BTreeMap<Key, (FootprintBounds, u64)>,
    counts: CacheCounts,
}

impl Inner {
    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// Drops the least recently used entries of `map` until it has at most
    /// `keep`.
    fn trim<V>(map: &mut BTreeMap<Key, (V, u64)>, keep: usize) {
        while map.len() > keep {
            let oldest = map
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| *k);
            match oldest {
                Some(k) => map.remove(&k),
                None => break,
            };
        }
    }
}

/// The shared cache of column footprints.
pub struct ColumnCache {
    /// Footprints kept at most; 0 keeps none (every load computes its own).
    capacity: usize,
    inner: Mutex<Inner>,
}

impl ColumnCache {
    /// A cache holding at most `cap_mib` MiB of columns.
    pub fn new(cap_mib: u32) -> Self {
        Self {
            capacity: ((cap_mib as usize) << 20) / FOOTPRINT_BYTES,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// The bounds of `chunk`'s footprint, if a load has computed them.
    pub fn bounds(&self, generator: &Generator, chunk: ChunkCoord) -> Option<FootprintBounds> {
        let mut inner = self.inner.lock().ok()?;
        let now = inner.tick();
        let entry = inner.bounds.get_mut(&key(generator, chunk))?;
        entry.1 = now;
        Some(entry.0)
    }

    /// The columns of `chunk`'s footprint (its `generator.columns`, possibly
    /// computed for another chunk of the footprint) and their bounds.
    pub fn columns(
        &self,
        generator: &Generator,
        chunk: ChunkCoord,
    ) -> (Arc<ChunkColumns>, FootprintBounds) {
        let k = key(generator, chunk);
        let slot = if self.capacity == 0 {
            None
        } else {
            self.inner.lock().ok().map(|mut inner| {
                let now = inner.tick();
                match inner.columns.get_mut(&k) {
                    Some(entry) => {
                        entry.1 = now;
                        Arc::clone(&entry.0)
                    }
                    None => {
                        let slot = Slot::default();
                        inner.columns.insert(k, (Arc::clone(&slot), now));
                        Inner::trim(&mut inner.columns, self.capacity);
                        slot
                    }
                }
            })
        };
        let mut computed = false;
        let columns = match &slot {
            Some(slot) => Arc::clone(slot.get_or_init(|| {
                computed = true;
                Arc::new(generator.columns(chunk))
            })),
            None => {
                computed = true;
                Arc::new(generator.columns(chunk))
            }
        };
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return (Arc::clone(&columns), columns.bounds()),
        };
        let now = inner.tick();
        let bounds = match inner.bounds.get_mut(&k) {
            Some(entry) => {
                entry.1 = now;
                entry.0
            }
            None => {
                let bounds = columns.bounds();
                inner.bounds.insert(k, (bounds, now));
                Inner::trim(&mut inner.bounds, MAX_BOUNDS);
                bounds
            }
        };
        if computed {
            inner.counts.misses += 1;
        } else {
            inner.counts.hits += 1;
        }
        (columns, bounds)
    }

    pub fn counts(&self) -> CacheCounts {
        self.inner.lock().map(|i| i.counts).unwrap_or_default()
    }

    /// Footprints held now, for tests.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().map(|i| i.columns.len()).unwrap_or(0)
    }

    /// The footprints held, for tests: their chunk `x` and `z`, least
    /// recently used first.
    #[cfg(test)]
    pub fn held(&self) -> Vec<(u32, u32)> {
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };
        let mut held: Vec<(u64, (u32, u32))> = inner
            .columns
            .iter()
            .map(|((_, x, z), (_, used))| (*used, (*x, *z)))
            .collect();
        held.sort_unstable();
        held.into_iter().map(|(_, c)| c).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_gen::{GeneratorVersion, TerrainConfig};

    fn generator() -> Generator {
        Generator::new(7, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap()
    }

    fn chunk(x: u32, y: u32, z: u32) -> ChunkCoord {
        ChunkCoord::new(x, y, z).unwrap()
    }

    #[test]
    fn stacked_chunks_share_one_footprint_and_the_budget_evicts_the_least_recently_used() {
        let g = generator();
        // 16 MiB: two footprints.
        let cache = ColumnCache::new(16);
        let (a, _) = cache.columns(&g, chunk(500, 510, 500));
        let (a2, bounds) = cache.columns(&g, chunk(500, 511, 500));
        assert!(
            Arc::ptr_eq(&a, &a2),
            "the second chunk of the column shares it"
        );
        assert_eq!(*a, g.columns(chunk(500, 510, 500)));
        assert_eq!(bounds, a.bounds());
        cache.columns(&g, chunk(501, 510, 500));
        cache.columns(&g, chunk(500, 509, 500)); // uses (500, 500) again
        cache.columns(&g, chunk(502, 510, 500)); // evicts (501, 500)
        assert_eq!(cache.held(), vec![(500, 500), (502, 500)]);
        assert_eq!(cache.counts(), CacheCounts { hits: 2, misses: 3 });
        // Bounds outlive their columns.
        assert!(cache.bounds(&g, chunk(501, 0, 500)).is_some());
        assert!(cache.bounds(&g, chunk(503, 0, 500)).is_none());
        // Another generator is another footprint.
        let other = Generator::new(8, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap();
        assert!(cache.bounds(&other, chunk(500, 0, 500)).is_none());
        let (b, _) = cache.columns(&other, chunk(500, 510, 500));
        assert_eq!(b.generator(), other.id());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn a_zero_budget_keeps_nothing() {
        let g = generator();
        let cache = ColumnCache::new(0);
        let (a, _) = cache.columns(&g, chunk(500, 510, 500));
        let (b, _) = cache.columns(&g, chunk(500, 511, 500));
        assert!(!Arc::ptr_eq(&a, &b));
        assert_eq!(a.columns(), b.columns());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.counts().misses, 2);
    }

    #[test]
    fn workers_asking_at_once_compute_a_footprint_once() {
        let g = Arc::new(generator());
        let cache = Arc::new(ColumnCache::new(64));
        let threads: Vec<_> = (0..4)
            .map(|y| {
                let (g, cache) = (Arc::clone(&g), Arc::clone(&cache));
                std::thread::spawn(move || cache.columns(&g, chunk(600, 500 + y, 600)).0)
            })
            .collect();
        let all: Vec<Arc<ChunkColumns>> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(all.iter().all(|c| Arc::ptr_eq(c, &all[0])));
        assert_eq!(cache.counts(), CacheCounts { hits: 3, misses: 1 });
    }
}
