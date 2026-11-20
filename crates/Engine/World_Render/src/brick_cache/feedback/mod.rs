//! Requests and victims coming back from the GPU.
//!
//! # Request block (start of the feedback buffer)
//!
//! `[count, overflow, 0, 0]`, then `capacity` requests of two words:
//! `x | y << 16` and `z | level << 16 | priority << 20`, where `x, y, z` is
//! the requested node's lowest corner in brick units (16 voxels) from the
//! tree origin, `level` the node's level (1 for a brick) and `priority` a
//! 12-bit hint, higher first. Rays dedupe with `atomicMax` on a per-node
//! stamp (the words after the requests), so each node is requested at most
//! once per frame; past `capacity` the counter keeps counting and `overflow`
//! counts the dropped requests.
//!
//! # Victim block
//!
//! `[count, overflow, 0, 0]`, then victims of two words: brick id and the
//! block ref the id held when it was chosen.
//!
//! Decoding is deterministic for given buffer contents and validates every
//! field: read-back data never panics the CPU.

pub mod ring;

use super::stream::key::{NodeKey, Shape};
use crate::brick_cache::pool::layout::{FEEDBACK_HEADER_WORDS, REQUEST_WORDS, VICTIM_WORDS};

pub const REQUEST_LEVEL_SHIFT: u32 = 16;
pub const REQUEST_PRIORITY_SHIFT: u32 = 20;
pub const REQUEST_PRIORITY_MAX: u32 = 0xFFF;

/// The request and victim layout constants the shaders mirror.
pub const SHADER_CONSTANTS: &[(&str, u32)] = &[
    ("FEEDBACK_HEADER_WORDS", FEEDBACK_HEADER_WORDS),
    ("REQUEST_WORDS", REQUEST_WORDS),
    ("VICTIM_WORDS", VICTIM_WORDS),
    ("REQUEST_LEVEL_SHIFT", REQUEST_LEVEL_SHIFT),
    ("REQUEST_PRIORITY_SHIFT", REQUEST_PRIORITY_SHIFT),
    ("REQUEST_PRIORITY_MAX", REQUEST_PRIORITY_MAX),
];

/// One node a ray wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub key: NodeKey,
    pub priority: u32,
}

/// One brick an eviction pass chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Victim {
    pub id: u32,
    pub block: u32,
}

/// Counters from one decoded block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockStats {
    /// Entries the GPU tried to write.
    pub written: u32,
    /// Entries dropped for lack of room.
    pub overflow: u32,
    /// Entries that failed validation.
    pub invalid: u32,
}

/// Encodes a request as the shader does (for tests).
pub fn encode_request(key: NodeKey, priority: u32) -> [u32; 2] {
    let scale = 2 * (key.level() - 1);
    let [x, y, z] = [key.x << scale, key.y << scale, key.z << scale];
    [
        x | y << 16,
        z | key.level() << REQUEST_LEVEL_SHIFT
            | priority.min(REQUEST_PRIORITY_MAX) << REQUEST_PRIORITY_SHIFT,
    ]
}

/// Decodes and dedupes a request block. Output is sorted by priority
/// descending, then by key.
pub fn decode_requests(words: &[u32], capacity: u32, shape: Shape) -> (Vec<Request>, BlockStats) {
    let written = words.first().copied().unwrap_or(0);
    let mut stats = BlockStats {
        written,
        overflow: words.get(1).copied().unwrap_or(0),
        invalid: 0,
    };
    let bricks_per_axis = (shape.size() / 16).min(1 << 16) as u32;
    let n = written.min(capacity) as usize;
    let mut out: Vec<Request> = Vec::with_capacity(n);
    for e in 0..n {
        let at = FEEDBACK_HEADER_WORDS as usize + e * REQUEST_WORDS as usize;
        let (Some(&w0), Some(&w1)) = (words.get(at), words.get(at + 1)) else {
            stats.invalid += 1;
            continue;
        };
        let (x, y, z) = (w0 & 0xFFFF, w0 >> 16, w1 & 0xFFFF);
        let level = (w1 >> REQUEST_LEVEL_SHIFT) & 0xF;
        let priority = w1 >> REQUEST_PRIORITY_SHIFT;
        let in_range = [x, y, z].iter().all(|&c| c < bricks_per_axis);
        if !(1..=shape.root_level()).contains(&level) || !in_range {
            stats.invalid += 1;
            continue;
        }
        let local = [x * 16, y * 16, z * 16];
        let key = NodeKey::containing(local, level);
        if key.min_local() != local {
            stats.invalid += 1; // not a node corner
            continue;
        }
        out.push(Request { key, priority });
    }
    out.sort_unstable_by(|a, b| a.key.cmp(&b.key).then(b.priority.cmp(&a.priority)));
    out.dedup_by_key(|r| r.key);
    out.sort_unstable_by(|a, b| b.priority.cmp(&a.priority).then(a.key.cmp(&b.key)));
    (out, stats)
}

/// Decodes a victim block, sorted and deduped.
pub fn decode_victims(words: &[u32], capacity: u32, brick_ids: u32) -> (Vec<Victim>, BlockStats) {
    let written = words.first().copied().unwrap_or(0);
    let mut stats = BlockStats {
        written,
        overflow: words.get(1).copied().unwrap_or(0),
        invalid: 0,
    };
    let n = written.min(capacity) as usize;
    let mut out = Vec::with_capacity(n);
    for e in 0..n {
        let at = FEEDBACK_HEADER_WORDS as usize + e * VICTIM_WORDS as usize;
        match (words.get(at), words.get(at + 1)) {
            (Some(&id), Some(&block)) if id < brick_ids => out.push(Victim { id, block }),
            _ => stats.invalid += 1,
        }
    }
    out.sort_unstable();
    out.dedup();
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::VoxelPos;

    fn shape() -> Shape {
        Shape {
            levels: 4,
            origin: VoxelPos::new(0, 0, 0),
        }
    }

    #[test]
    fn requests_round_trip_dedupe_and_rank() {
        let a = NodeKey {
            level: 1,
            x: 3,
            y: 1,
            z: 15,
        };
        let b = NodeKey {
            level: 2,
            x: 1,
            y: 0,
            z: 2,
        };
        let mut words = vec![3, 0, 0, 0];
        words.extend(encode_request(a, 5));
        words.extend(encode_request(b, 9));
        words.extend(encode_request(a, 7));
        let (reqs, stats) = decode_requests(&words, 16, shape());
        assert_eq!(stats.invalid, 0);
        assert_eq!(
            reqs,
            vec![
                Request {
                    key: b,
                    priority: 9
                },
                Request {
                    key: a,
                    priority: 7
                }
            ]
        );
    }

    #[test]
    fn garbage_is_counted_not_trusted() {
        // Count beyond the data, a bad level and an out-of-range coordinate.
        let words = vec![
            5,
            2,
            0,
            0,
            0,
            9 << REQUEST_LEVEL_SHIFT,
            0xFFFF,
            1 << REQUEST_LEVEL_SHIFT,
        ];
        let (reqs, stats) = decode_requests(&words, 64, shape());
        assert!(reqs.is_empty());
        assert_eq!((stats.written, stats.overflow, stats.invalid), (5, 2, 5));
    }

    #[test]
    fn victims_are_validated_and_sorted() {
        let words = vec![3, 0, 0, 0, 9, 100, 2, 50, 70000, 1];
        let (v, stats) = decode_victims(&words, 8, 1024);
        assert_eq!(
            v,
            vec![Victim { id: 2, block: 50 }, Victim { id: 9, block: 100 }]
        );
        assert_eq!(stats.invalid, 1);
    }
}
