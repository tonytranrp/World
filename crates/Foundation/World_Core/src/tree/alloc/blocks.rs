//! Word block allocation: blocks of 1 to `MAX_BLOCK_LEN` words, recycled by
//! exact size first, else cut from the shortest longer free block.

use std::sync::Arc;

use super::MAX_BLOCK_LEN;
use super::pages::Pages;

/// Which word blocks are in use. The words themselves live in the pages.
///
/// New blocks are cut from the end of the last page. A block never straddles
/// a page, so when the next block does not fit, the rest of the page (always
/// shorter than the block, so under 192 words) becomes a free block of its own
/// size and the new block starts the next page. Freed blocks go on a list
/// per size; a request takes a block of exactly its size if there is one
/// (the lowest of a short list, the last freed of a long one), else the
/// front of the shortest longer one, whose rest stays free, and only then
/// cuts from the end. So edits fill holes from the front of the arena
/// instead of growing it, and the small blocks every lookup reads (child
/// words of the upper levels) do not drift to the last pages (decision
/// 0035).
#[derive(Clone, Default)]
pub(super) struct WordBlocks {
    /// One past the highest word ever handed out.
    end: u32,
    /// `free[len - 1]` holds the bases of free blocks of `len` words, or is
    /// `None` when there are none; lengths past the end have none. Each list
    /// is shared until written, so cloning the allocator (the first write to
    /// a chunk after a snapshot) copies reference counts, and a write then
    /// copies only the list it takes from or adds to. Grown only as far as
    /// the longest freed length, so a built chunk (whose only free blocks
    /// are its page tails) does not hold a slot for each of the
    /// `MAX_BLOCK_LEN` lengths, and a lookup is one index, not a search.
    free: Vec<Option<Arc<Vec<u32>>>>,
    /// Bit `len - 1` is set when `free[len - 1]` holds a block.
    nonempty: [u64; FREE_BITS],
    live_words: usize,
    free_words: usize,
}

/// Free lists up to this long hand out their lowest block; longer ones the
/// last freed.
const LOWEST_FIRST_MAX: usize = 16;

/// Words of the bitmap of lengths with free blocks.
const FREE_BITS: usize = MAX_BLOCK_LEN.div_ceil(64);

impl WordBlocks {
    /// The base of a block of `len` words, 1 to `MAX_BLOCK_LEN`, inside one
    /// page: a free block of exactly that length, else the front of the
    /// shortest longer free block (the rest stays free), else a new block at
    /// the end. Panics on any other length or when word offsets would
    /// overflow `u32`.
    pub fn alloc<const N: usize>(&mut self, len: usize, pages: &mut Pages<u32, N>) -> u32 {
        assert!(
            (1..=MAX_BLOCK_LEN).contains(&len),
            "a word block holds 1 to {MAX_BLOCK_LEN} words, not {len}"
        );
        let base = match self.take_free(len) {
            Some(base) => {
                self.free_words -= len;
                base
            }
            None => match self.shortest_free_above(len) {
                // Splitting a longer block keeps the arena from growing
                // while holes it could fill wait (decision 0035).
                Some(longer) => {
                    let base = self
                        .take_free(longer)
                        .expect("the bitmap names a free block");
                    self.push_free(longer - len, base + len as u32);
                    self.free_words -= len;
                    base
                }
                None => self.cut(len, pages),
            },
        };
        self.live_words += len;
        base
    }

    /// The shortest length above `len` with a free block.
    fn shortest_free_above(&self, len: usize) -> Option<usize> {
        // Bit `len` stands for length `len + 1`.
        let (mut word, bit) = (len / 64, len % 64);
        if word >= FREE_BITS {
            return None;
        }
        let mut bits = self.nonempty[word] & (u64::MAX << bit);
        loop {
            if bits != 0 {
                return Some(word * 64 + bits.trailing_zeros() as usize + 1);
            }
            word += 1;
            if word == FREE_BITS {
                return None;
            }
            bits = self.nonempty[word];
        }
    }

    /// A free block of exactly `len` words, if there is one; the last freed
    /// first. A length whose list empties keeps its list (and the list's
    /// allocation) for the next block of that length.
    fn take_free(&mut self, len: usize) -> Option<u32> {
        let list = self.free.get_mut(len - 1)?.as_mut()?;
        if list.is_empty() {
            return None;
        }
        let list = Arc::make_mut(list);
        // The lowest of a short list, so blocks fill the arena from its
        // front and blocks read together stay on few pages; the last freed
        // of a long one, which costs no search.
        let base = if list.len() <= LOWEST_FIRST_MAX {
            let (at, _) = list
                .iter()
                .enumerate()
                .min_by_key(|&(_, base)| *base)
                .expect("the list is not empty");
            Some(list.swap_remove(at))
        } else {
            list.pop()
        };
        if list.is_empty() {
            self.nonempty[(len - 1) / 64] &= !(1 << ((len - 1) % 64));
        }
        base
    }

    /// Changes the length of the block at `base` from `old_len` to
    /// `new_len` words in place when it is the last block cut from the arena
    /// and the new length still fits its page, and returns whether it did.
    /// A block that grows edit after edit (a brick being built voxel by
    /// voxel) then keeps its place instead of leaving a free block of every
    /// length it had and scattering over the pages.
    pub fn resize_last<const N: usize>(
        &mut self,
        base: u32,
        old_len: usize,
        new_len: usize,
    ) -> bool {
        let fits = (base as usize % N) + new_len <= N;
        if base as usize + old_len != self.end as usize || !fits || new_len == 0 {
            return false;
        }
        self.end = base + new_len as u32;
        self.live_words = self.live_words + new_len - old_len;
        true
    }

    /// Returns a block for reuse. Its page is not written.
    pub fn free(&mut self, base: u32, len: usize) {
        debug_assert!(
            base as usize + len <= self.end as usize,
            "block was never allocated"
        );
        self.push_free(len, base);
        self.live_words -= len;
        self.free_words += len;
    }

    fn push_free(&mut self, len: usize, base: u32) {
        if self.free.len() < len {
            self.free.resize(len, None);
        }
        Arc::make_mut(self.free[len - 1].get_or_insert_with(Default::default)).push(base);
        self.nonempty[(len - 1) / 64] |= 1 << ((len - 1) % 64);
    }

    /// One past the highest word ever handed out.
    pub fn end(&self) -> usize {
        self.end as usize
    }

    pub fn live_words(&self) -> usize {
        self.live_words
    }

    pub fn free_words(&self) -> usize {
        self.free_words
    }

    /// Cuts a fresh block from the end of the arena.
    fn cut<const N: usize>(&mut self, len: usize, pages: &mut Pages<u32, N>) -> u32 {
        let offset = self.end as usize % N;
        if offset != 0 && offset + len > N {
            let rest = N - offset;
            self.push_free(rest, self.end);
            self.free_words += rest;
            self.end += rest as u32;
        }
        if self.end as usize == pages.page_count() * N {
            pages.push_page();
        }
        let base = self.end;
        self.end = u32::try_from(len)
            .ok()
            .and_then(|len| base.checked_add(len))
            .expect("the word arena is full (2^32 words)");
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: usize = 512;

    fn arena() -> (WordBlocks, Pages<u32, PAGE>) {
        (WordBlocks::default(), Pages::default())
    }

    #[test]
    fn freed_blocks_are_reused_by_exact_size_then_split() {
        let (mut blocks, mut pages) = arena();
        let a = blocks.alloc(3, &mut pages);
        let b = blocks.alloc(5, &mut pages);
        blocks.free(a, 3);
        assert_eq!(
            blocks.alloc(4, &mut pages),
            b + 5,
            "a 3-word hole does not fit 4"
        );
        assert_eq!(blocks.alloc(3, &mut pages), a, "an exact fit");
        blocks.free(b, 5);
        // No 2-word hole: the front of the 5-word one; its rest stays free.
        assert_eq!(blocks.alloc(2, &mut pages), b);
        assert_eq!(blocks.free_words(), 3);
        assert_eq!(blocks.alloc(3, &mut pages), b + 2);
        assert_eq!(blocks.live_words(), 12);
        assert_eq!(blocks.free_words(), 0);
    }

    #[test]
    fn reuse_takes_the_lowest_free_block_of_a_size_first() {
        let (mut blocks, mut pages) = arena();
        let a = blocks.alloc(2, &mut pages);
        let b = blocks.alloc(2, &mut pages);
        blocks.free(a, 2);
        blocks.free(b, 2);
        assert_eq!(blocks.alloc(2, &mut pages), a);
        assert_eq!(blocks.alloc(2, &mut pages), b);
    }

    #[test]
    fn the_last_block_resizes_in_place() {
        let (mut blocks, mut pages) = arena();
        let a = blocks.alloc(4, &mut pages);
        let b = blocks.alloc(4, &mut pages);
        assert!(!blocks.resize_last::<PAGE>(a, 4, 6), "not the last block");
        assert!(blocks.resize_last::<PAGE>(b, 4, 9));
        assert_eq!(blocks.end(), b as usize + 9);
        assert!(blocks.resize_last::<PAGE>(b, 9, 2));
        assert_eq!((blocks.end(), blocks.live_words()), (b as usize + 2, 6));
        assert!(!blocks.resize_last::<PAGE>(b, 2, PAGE), "past its page");
    }

    #[test]
    fn blocks_never_straddle_a_page_and_the_rest_is_recycled() {
        let (mut blocks, mut pages) = arena();
        let bases: Vec<u32> = (0..5).map(|_| blocks.alloc(120, &mut pages)).collect();
        assert_eq!(bases[..4], [0, 120, 240, 360]);
        assert_eq!(
            bases[4], PAGE as u32,
            "the fifth block starts the next page"
        );
        assert_eq!(pages.page_count(), 2);
        assert_eq!(blocks.free_words(), 32);
        assert_eq!(
            blocks.alloc(32, &mut pages),
            480,
            "the page's tail is reused"
        );
        assert_eq!(blocks.free_words(), 0);
    }

    #[test]
    fn every_block_lies_in_one_page() {
        let (mut blocks, mut pages) = arena();
        for i in 0..500 {
            let len = 1 + (i * 37) % MAX_BLOCK_LEN;
            let base = blocks.alloc(len, &mut pages) as usize;
            assert_eq!(base / PAGE, (base + len - 1) / PAGE, "block {base}+{len}");
            assert!(base + len <= pages.page_count() * PAGE);
        }
    }

    #[test]
    fn allocation_is_deterministic() {
        let run = || {
            let (mut blocks, mut pages) = arena();
            let mut bases = Vec::new();
            for i in 0..200 {
                let len = 1 + (i * 13) % MAX_BLOCK_LEN;
                let base = blocks.alloc(len, &mut pages);
                bases.push(base);
                if i % 3 == 0 {
                    blocks.free(base, len);
                }
            }
            bases
        };
        assert_eq!(run(), run());
    }

    #[test]
    #[should_panic(expected = "1 to 327 words")]
    fn oversized_blocks_are_refused() {
        let (mut blocks, mut pages) = arena();
        blocks.alloc(MAX_BLOCK_LEN + 1, &mut pages);
    }
}
