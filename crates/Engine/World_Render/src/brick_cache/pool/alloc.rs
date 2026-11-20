//! CPU-side slot bookkeeping: brick space, node child groups and brick ids.
//!
//! The GPU never allocates; these structures are authoritative for what
//! occupies the pools.
//!
//! `offset-allocator` sizing: an `Allocator` over `n` units files its single
//! free region under the size class `n` rounds *down* to, and a request for
//! `s` units searches classes from `s` rounded *up*. Small requests can
//! therefore use every unit, but one request close to the free size can fail;
//! and the allocator tracks at most `max_allocs` live allocations, 128 Ki by
//! default, which a 2 GiB pool of ~1 KiB bricks would exceed many times over.
//! It also keeps two of its `max_allocs` bookkeeping nodes for itself (the
//! initial region and the remainder of a split), so `N` slots hold at most
//! `N - 2` live allocations. Each segment's allocator is therefore created over exactly the segment's
//! units with `max_allocs` sized from the expected mean block (see
//! `PoolLayout`), and the tests fill whole segments to prove the capacity is
//! reachable.

use offset_allocator::{Allocation, Allocator};

use crate::brick_cache::format::{BLOCK_ALIGN_WORDS, BrickRef};

/// Bookkeeping slots `offset-allocator` keeps for itself (see the module docs).
const ALLOCATOR_RESERVED: u32 = 2;

/// One brick block's place in the pool.
#[derive(Clone, Copy)]
pub struct BrickAlloc {
    pub block: BrickRef,
    /// Size in 16-byte units.
    pub units: u32,
    allocation: Allocation,
}

impl std::fmt::Debug for BrickAlloc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrickAlloc")
            .field("block", &self.block)
            .field("units", &self.units)
            .finish()
    }
}

struct Segment {
    allocator: Allocator,
    units: u32,
    used: u64,
}

/// The brick pool's space, segment by segment, in 16-byte units.
pub struct BrickSpace {
    segments: Vec<Segment>,
}

impl BrickSpace {
    /// Space for segments of the given byte sizes (multiples of 16, at most
    /// 4 GiB each), each tracking up to `max_allocs` blocks.
    pub fn new(segment_bytes: &[u64], max_allocs: u32) -> Self {
        let segments = segment_bytes
            .iter()
            .map(|&bytes| {
                let units = (bytes / 16).min(u64::from(u32::MAX - 1)) as u32;
                Segment {
                    allocator: Allocator::with_max_allocs(
                        units,
                        max_allocs
                            .saturating_add(ALLOCATOR_RESERVED)
                            .min(u32::MAX - 2),
                    ),
                    units,
                    used: 0,
                }
            })
            .collect();
        Self { segments }
    }

    /// Space for a block of `words` words, in the first segment that has
    /// room. `None` when the pool is full (or out of allocation slots).
    pub fn allocate(&mut self, words: u32) -> Option<BrickAlloc> {
        let units = words.div_ceil(BLOCK_ALIGN_WORDS).max(1);
        for (segment, s) in self.segments.iter_mut().enumerate() {
            if let Some(allocation) = s.allocator.allocate(units) {
                s.used += u64::from(units);
                return Some(BrickAlloc {
                    block: BrickRef {
                        segment: segment as u32,
                        offset16: allocation.offset,
                    },
                    units,
                    allocation,
                });
            }
        }
        None
    }

    /// Returns a block's space. Each allocation must be freed exactly once.
    pub fn free(&mut self, alloc: BrickAlloc) {
        if let Some(s) = self.segments.get_mut(alloc.block.segment as usize) {
            s.allocator.free(alloc.allocation);
            s.used -= u64::from(alloc.units);
        }
    }

    pub fn used_bytes(&self) -> u64 {
        self.segments.iter().map(|s| s.used * 16).sum()
    }

    pub fn capacity_bytes(&self) -> u64 {
        self.segments.iter().map(|s| u64::from(s.units) * 16).sum()
    }

    /// Used fraction, 0 to 1.
    pub fn fill(&self) -> f32 {
        let cap = self.capacity_bytes();
        if cap == 0 {
            1.0
        } else {
            (self.used_bytes() as f64 / cap as f64) as f32
        }
    }
}

/// One child group's place in the node pool.
#[derive(Clone, Copy)]
pub struct NodeAlloc {
    pub first: u32,
    pub len: u32,
    allocation: Allocation,
}

impl std::fmt::Debug for NodeAlloc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeAlloc")
            .field("first", &self.first)
            .field("len", &self.len)
            .finish()
    }
}

/// The node pool's space. Index 0 holds the root record and is never handed out.
pub struct NodeSpace {
    allocator: Allocator,
    capacity: u32,
    used: u32,
    _root: Allocation,
}

impl NodeSpace {
    /// Space for `capacity` records tracking up to `max_groups` child groups.
    /// `None` if the allocator does not place its first allocation at
    /// index 0, which the root relies on.
    pub fn new(capacity: u32, max_groups: u32) -> Option<Self> {
        let mut allocator = Allocator::with_max_allocs(
            capacity.max(2),
            max_groups
                .saturating_add(ALLOCATOR_RESERVED + 1)
                .min(u32::MAX - 2),
        );
        let root = allocator.allocate(1)?;
        (root.offset == 0).then_some(Self {
            allocator,
            capacity,
            used: 1,
            _root: root,
        })
    }

    /// A contiguous group of `len` records (1 to 64).
    pub fn allocate(&mut self, len: u32) -> Option<NodeAlloc> {
        let allocation = self.allocator.allocate(len.max(1))?;
        self.used += len.max(1);
        Some(NodeAlloc {
            first: allocation.offset,
            len: len.max(1),
            allocation,
        })
    }

    pub fn free(&mut self, alloc: NodeAlloc) {
        self.allocator.free(alloc.allocation);
        self.used -= alloc.len;
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn fill(&self) -> f32 {
        self.used as f32 / self.capacity.max(1) as f32
    }
}

/// Brick ids: indices into the brick table. Lowest free id first.
pub struct IdSpace {
    free: Vec<u32>,
    capacity: u32,
}

impl IdSpace {
    pub fn new(capacity: u32) -> Self {
        Self {
            free: (0..capacity).rev().collect(),
            capacity,
        }
    }

    pub fn allocate(&mut self) -> Option<u32> {
        self.free.pop()
    }

    pub fn free(&mut self, id: u32) {
        self.free.push(id);
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn in_use(&self) -> u32 {
        self.capacity - self.free.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_blocks_fill_a_segment_completely() {
        let mut space = BrickSpace::new(&[64 * 1024], 1 << 16);
        let mut blocks = Vec::new();
        while let Some(b) = space.allocate(4) {
            blocks.push(b);
        }
        assert_eq!(space.used_bytes(), space.capacity_bytes());
        for b in blocks {
            space.free(b);
        }
        assert_eq!(space.used_bytes(), 0);
        // After freeing everything the whole segment is one region again.
        assert!(space.allocate(64 * 1024 / 4 - 4).is_some());
    }

    #[test]
    fn brick_sized_blocks_use_nearly_all_of_every_segment() {
        // Mixed block sizes typical of bricks (48 B to 13 KiB) fill a pool of
        // three segments to within one largest block per segment.
        let segs = [1 << 20, 1 << 20, 512 << 10];
        let mut space = BrickSpace::new(&segs, 1 << 16);
        let sizes = [12u32, 40, 96, 260, 800, 3332, 64, 128];
        let mut i = 0;
        let mut failures = 0;
        while failures < sizes.len() {
            if space.allocate(sizes[i % sizes.len()]).is_some() {
                failures = 0;
            } else {
                failures += 1;
            }
            i += 1;
        }
        let slack = space.capacity_bytes() - space.used_bytes();
        assert!(
            slack <= 3 * 3332 * 4,
            "{slack} bytes left unusable of {}",
            space.capacity_bytes()
        );
    }

    #[test]
    fn allocation_slots_are_sized_for_the_mean_block() {
        // The default allocator would stop at 128 Ki blocks; ours tracks as
        // many as the segment holds at the configured mean block size.
        let bytes = 64u64 << 20;
        let mean = 256u64;
        let mut space = BrickSpace::new(&[bytes], (bytes / mean) as u32);
        let mut n = 0u64;
        while space.allocate((mean / 4) as u32).is_some() {
            n += 1;
        }
        assert_eq!(n, bytes / mean);
        assert!(n > 128 * 1024);
    }

    #[test]
    fn the_root_record_is_reserved() {
        let mut nodes = NodeSpace::new(1024, 256).unwrap();
        let g = nodes.allocate(8).unwrap();
        assert!(g.first >= 1);
        assert_eq!(nodes.used(), 9);
        nodes.free(g);
        assert_eq!(nodes.used(), 1);
    }

    #[test]
    fn ids_come_lowest_first_and_return() {
        let mut ids = IdSpace::new(4);
        assert_eq!(ids.allocate(), Some(0));
        assert_eq!(ids.allocate(), Some(1));
        ids.free(0);
        assert_eq!(ids.allocate(), Some(0));
        assert_eq!(ids.in_use(), 2);
    }
}
