//! Word block allocation: blocks of 1 to 192 words, recycled by exact size.

use std::sync::Arc;

use super::MAX_BLOCK_LEN;
use super::pages::Pages;

/// Which word blocks are in use. The words themselves live in the pages.
///
/// New blocks are cut from the end of the last page. A block never straddles
/// a page, so when the next block does not fit, the rest of the page (always
/// shorter than the block, so under 192 words) becomes a free block of its own
/// size and the new block starts the next page. Freed blocks go on a LIFO
/// list per size and are reused only by a request of exactly that size.
#[derive(Clone, Default)]
pub(super) struct WordBlocks {
    /// One past the highest word ever handed out.
    end: u32,
    /// The free blocks by length, ascending: `(len, bases)` for each length
    /// that has any. Each list is shared until written, so cloning the
    /// allocator (the first write to a chunk after a snapshot) copies
    /// reference counts, and a write then copies only the list it takes
    /// from or adds to. Sparse, so a chunk with few free sizes (a built
    /// chunk has only its page tails) does not hold a slot for every one of
    /// the `MAX_BLOCK_LEN` sizes.
    free: Vec<(u32, Arc<Vec<u32>>)>,
    live_words: usize,
    free_words: usize,
}

impl WordBlocks {
    /// The base of a block of `len` words, 1 to 192, inside one page. Panics
    /// on any other length or when word offsets would overflow `u32`.
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
            None => self.cut(len, pages),
        };
        self.live_words += len;
        base
    }

    /// A free block of exactly `len` words, if there is one; the last freed
    /// first. A length whose list empties keeps its entry (and the list's
    /// allocation) for the next block of that length.
    fn take_free(&mut self, len: usize) -> Option<u32> {
        let at = self
            .free
            .binary_search_by_key(&(len as u32), |&(l, _)| l)
            .ok()?;
        if self.free[at].1.is_empty() {
            return None;
        }
        Arc::make_mut(&mut self.free[at].1).pop()
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
        match self.free.binary_search_by_key(&(len as u32), |&(l, _)| l) {
            Ok(at) => Arc::make_mut(&mut self.free[at].1).push(base),
            Err(at) => self.free.insert(at, (len as u32, Arc::new(vec![base]))),
        }
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
    fn freed_blocks_are_reused_by_exact_size_only() {
        let (mut blocks, mut pages) = arena();
        let a = blocks.alloc(3, &mut pages);
        let b = blocks.alloc(5, &mut pages);
        blocks.free(a, 3);
        assert_eq!(
            blocks.alloc(4, &mut pages),
            b + 5,
            "a 3-word hole does not fit 4"
        );
        assert_eq!(blocks.alloc(3, &mut pages), a);
        assert_eq!(blocks.live_words(), 12);
        assert_eq!(blocks.free_words(), 0);
    }

    #[test]
    fn reuse_is_last_in_first_out() {
        let (mut blocks, mut pages) = arena();
        let a = blocks.alloc(2, &mut pages);
        let b = blocks.alloc(2, &mut pages);
        blocks.free(a, 2);
        blocks.free(b, 2);
        assert_eq!(blocks.alloc(2, &mut pages), b);
        assert_eq!(blocks.alloc(2, &mut pages), a);
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
