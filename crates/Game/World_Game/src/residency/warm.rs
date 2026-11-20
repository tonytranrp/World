//! The warm tier (decision 0006): LZ4-compressed chunks kept in RAM after
//! they leave the hot tier, so returning to them decompresses instead of
//! reading a region file or generating them again.

use std::collections::BTreeMap;

use world_gen::ChunkCoord;

use super::jobs::WarmChunk;

/// One compressed chunk in the warm cache.
struct WarmEntry {
    chunk: WarmChunk,
    /// Its content has saved edits, so losing it costs a region file read
    /// and an overlay merge on top of generation; it ages more slowly.
    edited: bool,
    /// The tick of its last use.
    used: u64,
}

/// Compressed chunks: edited chunks as their region files hold them, and
/// unedited chunks as generated. Every entry is what loading the chunk from
/// disk or the generator would give, so it stays valid until the chunk is
/// written again, which replaces it.
///
/// Eviction takes the entry with the greatest age since its last use, where
/// edited entries age `edited_weight` times more slowly, ties to the lowest
/// coordinate. `bytes` is the exact sum of the entries' sizes and never
/// exceeds the cap after an insert.
#[derive(Default)]
pub(super) struct Warm {
    entries: BTreeMap<ChunkCoord, WarmEntry>,
    bytes: usize,
    tick: u64,
}

impl Warm {
    /// Stores `chunk` for `coord`, replacing any entry, then evicts until
    /// the bytes fit `cap`. A chunk larger than `cap` is not kept (and an
    /// old entry for `coord` is dropped all the same). Returns whether it
    /// was kept.
    pub(super) fn insert(
        &mut self,
        coord: ChunkCoord,
        chunk: WarmChunk,
        edited: bool,
        cap: usize,
        edited_weight: f32,
    ) -> bool {
        self.remove(coord);
        if chunk.size() > cap {
            return false;
        }
        self.tick += 1;
        self.bytes += chunk.size();
        let used = self.tick;
        self.entries.insert(
            coord,
            WarmEntry {
                chunk,
                edited,
                used,
            },
        );
        self.shrink_to(cap, edited_weight);
        true
    }

    /// Evicts entries until the bytes fit `cap`.
    pub(super) fn shrink_to(&mut self, cap: usize, edited_weight: f32) {
        while self.bytes > cap {
            let Some(victim) = self.victim(edited_weight) else {
                break;
            };
            self.remove(victim);
        }
    }

    /// The entry to evict first: the greatest weighted age.
    fn victim(&self, edited_weight: f32) -> Option<ChunkCoord> {
        let weight = f64::from(edited_weight.max(1.0));
        let mut best: Option<(f64, ChunkCoord)> = None;
        for (&coord, e) in &self.entries {
            let age = (self.tick - e.used) as f64;
            let score = if e.edited { age / weight } else { age };
            // Strictly greater: ties keep the lowest coordinate.
            if best.is_none_or(|(b, _)| score > b) {
                best = Some((score, coord));
            }
        }
        best.map(|(_, c)| c)
    }

    pub(super) fn remove(&mut self, coord: ChunkCoord) -> Option<WarmChunk> {
        let old = self.entries.remove(&coord)?;
        self.bytes -= old.chunk.size();
        Some(old.chunk)
    }

    /// The entry for `coord` (sharing its compressed bytes), which becomes
    /// the most recently used. The entry stays.
    pub(super) fn get(&mut self, coord: ChunkCoord) -> Option<WarmChunk> {
        let e = self.entries.get_mut(&coord)?;
        self.tick += 1;
        e.used = self.tick;
        Some(e.chunk.clone())
    }

    /// Marks the entry for `coord` as just used; whether there is one.
    pub(super) fn touch(&mut self, coord: ChunkCoord) -> bool {
        self.get(coord).is_some()
    }

    #[cfg(test)]
    pub(super) fn contains(&self, coord: ChunkCoord) -> bool {
        self.entries.contains_key(&coord)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Bytes held, kept up to date on every change.
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Entries holding saved edits.
    pub(super) fn edited(&self) -> usize {
        self.entries.values().filter(|e| e.edited).count()
    }

    /// Bytes held, summed afresh.
    #[cfg(test)]
    pub(super) fn bytes_walked(&self) -> usize {
        self.entries.values().map(|e| e.chunk.size()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(size: usize) -> WarmChunk {
        WarmChunk {
            bytes: vec![7; size].into(),
            len: size * 2,
        }
    }

    fn at(i: u32) -> ChunkCoord {
        ChunkCoord::new(i, 0, 0).unwrap()
    }

    #[test]
    fn the_least_recently_used_goes_first() {
        let mut warm = Warm::default();
        for i in 0..3 {
            assert!(warm.insert(at(i), chunk(100), false, 300, 4.0));
        }
        assert!(warm.touch(at(0)));
        warm.insert(at(3), chunk(100), false, 300, 4.0);
        assert!(warm.contains(at(0)), "used a moment ago");
        assert!(!warm.contains(at(1)), "the oldest");
        assert!(warm.contains(at(2)) && warm.contains(at(3)));
        assert_eq!(warm.bytes(), 300);
    }

    /// The tick at which an edited entry, inserted first into a cache of
    /// three, is evicted while unedited entries keep arriving.
    fn edited_survives_until(weight: f32) -> u32 {
        // The edited entry has the highest coordinate, so it loses no tie.
        let edited = at(500);
        let mut warm = Warm::default();
        warm.insert(edited, chunk(10), true, 30, weight);
        for i in 1..100 {
            warm.insert(at(i), chunk(10), false, 30, weight);
            if !warm.contains(edited) {
                return i;
            }
        }
        panic!("never evicted");
    }

    #[test]
    fn edited_entries_age_more_slowly() {
        // Unweighted it is simply the oldest when the fourth entry arrives.
        assert_eq!(edited_survives_until(1.0), 3);
        // Weighted 4, it goes once its age over 4 passes the oldest unedited
        // entry's age, which stays 2 in a cache of three: age 9.
        assert_eq!(edited_survives_until(4.0), 9);
        assert!(edited_survives_until(16.0) > edited_survives_until(4.0));
    }

    #[test]
    fn ties_go_to_the_lowest_coordinate() {
        // An edited entry of age 4 and an unedited one of age 2 tie at
        // weight 2; a fixed rule picks the victim, so every run agrees.
        let tie = |edited: u32, unedited: u32| {
            let mut warm = Warm::default();
            warm.insert(at(edited), chunk(10), true, 30, 2.0);
            warm.insert(at(8), chunk(10), false, 30, 2.0);
            warm.insert(at(unedited), chunk(10), false, 30, 2.0);
            warm.touch(at(8));
            warm.insert(at(9), chunk(10), false, 30, 2.0);
            assert_eq!(warm.len(), 3);
            (warm.contains(at(edited)), warm.contains(at(unedited)))
        };
        assert_eq!(tie(7, 4), (true, false));
        assert_eq!(tie(3, 4), (false, true));
    }

    #[test]
    fn a_chunk_larger_than_the_cap_is_not_kept() {
        let mut warm = Warm::default();
        warm.insert(at(0), chunk(50), false, 100, 4.0);
        assert!(!warm.insert(at(0), chunk(101), false, 100, 4.0));
        assert!(
            !warm.contains(at(0)),
            "the old copy is stale, so it goes too"
        );
        assert_eq!(warm.bytes(), 0);
        assert!(!warm.insert(at(1), chunk(1), false, 0, 4.0), "a zero cap");
    }

    #[test]
    fn the_byte_count_is_exact_and_within_the_cap() {
        // A fixed pseudo-random mix of inserts, replacements, uses, removals
        // and cap changes.
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let mut warm = Warm::default();
        let mut cap = 10_000;
        for step in 0..20_000 {
            let coord = at(next(64) as u32);
            match next(10) {
                0..=4 => {
                    let size = 1 + next(3000) as usize;
                    let kept = warm.insert(coord, chunk(size), next(3) == 0, cap, 4.0);
                    assert_eq!(kept, size <= cap, "step {step}");
                    assert_eq!(kept, warm.contains(coord), "step {step}");
                }
                5..=7 => {
                    warm.touch(coord);
                }
                8 => {
                    warm.remove(coord);
                }
                _ => {
                    cap = next(20_000) as usize;
                    warm.shrink_to(cap, 4.0);
                }
            }
            assert_eq!(warm.bytes(), warm.bytes_walked(), "step {step}");
            assert!(warm.bytes() <= cap, "step {step}");
        }
    }
}
