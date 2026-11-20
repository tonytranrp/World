//! Choosing what leaves the brick pool.
//!
//! When the pool is fuller than the threshold, one compute pass
//! (`assets/shaders/brick_cache/evict.wesl`, a single 256-thread workgroup)
//! builds a 256-bin histogram of brick ages (`frame - last_used`, the last
//! bin collecting everything older) in workgroup memory, picks the youngest
//! age `T >= min_age` such that at least `target` bricks are `T` or older,
//! and compacts the ids of those bricks into the victim block with their
//! block refs. The CPU frees them when the read-back arrives, after checking
//! each id still holds the same block. Data never moves on the GPU except by
//! upload.
//!
//! Which bricks are chosen is determined by the stamps (ages), not by
//! thread timing; only their order in the block varies, and the CPU sorts.

pub mod pass;

use bytemuck::{Pod, Zeroable};

/// Age histogram bins.
pub const AGE_BINS: u32 = 256;
/// Threads in the eviction workgroup.
pub const EVICT_WORKGROUP: u32 = 256;

/// The eviction pass's uniform block.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct EvictParams {
    pub frame: u32,
    /// Bricks used within this many frames are never chosen.
    pub min_age: u32,
    /// Victims wanted.
    pub target: u32,
    /// Brick ids to scan.
    pub id_count: u32,
    /// Victims the block holds.
    pub capacity: u32,
    pub _pad: [u32; 3],
}

/// How many bricks one pass should try to free, if any: none while the pool
/// is at or below `threshold`, else `fraction` of the resident bricks,
/// rounded (at least one), at most `capacity`.
pub fn eviction_target(
    fill: f32,
    threshold: f32,
    fraction: f32,
    resident: usize,
    capacity: u32,
) -> Option<u32> {
    if fill <= threshold || resident == 0 {
        return None;
    }
    let n = ((resident as f64 * f64::from(fraction)).round() as u32).max(1);
    Some(n.min(capacity))
}

/// The CPU model of the pass, for tests: the ids it chooses, sorted.
/// `table` holds `[block ref, last_used]` per id.
pub fn choose_victims(table: &[u32], params: &EvictParams) -> Vec<u32> {
    let ids = (params.id_count as usize).min(table.len() / 2);
    let age = |id: usize| -> Option<u32> {
        let (block, last) = (table[2 * id], table[2 * id + 1]);
        (last != 0 && block != u32::MAX).then(|| params.frame.wrapping_sub(last).min(AGE_BINS - 1))
    };
    let mut hist = [0u32; AGE_BINS as usize];
    for id in 0..ids {
        if let Some(a) = age(id) {
            hist[a as usize] += 1;
        }
    }
    let min_age = params.min_age.min(AGE_BINS - 1);
    let mut threshold = min_age;
    let mut acc = 0u32;
    for t in (min_age..AGE_BINS).rev() {
        acc += hist[t as usize];
        if acc >= params.target {
            threshold = t;
            break;
        }
    }
    let mut out: Vec<u32> = (0..ids)
        .filter(|&id| age(id).is_some_and(|a| a >= threshold))
        .map(|id| id as u32)
        .collect();
    out.truncate(params.capacity as usize);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pass_below_the_threshold() {
        assert_eq!(eviction_target(0.5, 0.9, 0.05, 1000, 64), None);
        assert_eq!(eviction_target(0.95, 0.9, 0.05, 1000, 64), Some(50));
        assert_eq!(eviction_target(0.95, 0.9, 0.05, 10_000, 64), Some(64));
        assert_eq!(eviction_target(0.95, 0.9, 0.05, 3, 64), Some(1));
    }

    #[test]
    fn the_oldest_go_first_and_the_young_never() {
        // ids 0..4 last used at frames 100, 10, 50, 0 (free), 95; now 200.
        let table = [1, 100, 2, 10, 3, 50, u32::MAX, 0, 5, 195];
        let p = EvictParams {
            frame: 200,
            min_age: 30,
            target: 1,
            id_count: 5,
            capacity: 8,
            ..Default::default()
        };
        assert_eq!(choose_victims(&table, &p), vec![1]);
        let all = EvictParams { target: 100, ..p };
        // Id 4 is 5 frames old and id 3 is free: never chosen.
        assert_eq!(choose_victims(&table, &all), vec![0, 1, 2]);
    }
}
