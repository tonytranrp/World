//! Copy-on-write paged storage behind a two-level directory.

use std::sync::Arc;

/// Pages per directory chunk.
///
/// The first write after a snapshot copies the directory (one `Arc` per
/// chunk) and every chunk it touches (one `Arc` per page), never the whole
/// page list. With 16 KiB pages a chunk spans 4 MiB, so a GiB of storage is
/// 256 chunks: such a write costs a few hundred reference-count increments
/// instead of the 65,536 a flat page list would.
pub(super) const CHUNK_PAGES: usize = 256;

/// One page of items: `N` of them, or fewer for a last page that was
/// trimmed to what it holds (see [`Pages::trim`]).
type Page<T> = Arc<[T]>;

/// Up to [`CHUNK_PAGES`] consecutive pages.
type Chunk<T> = Arc<[Page<T>]>;

/// A growable array of `T` in fixed pages of `N` items.
///
/// Pages are grouped in chunks of [`CHUNK_PAGES`]; every chunk but the last
/// is full. Each page, each chunk and the directory are behind an `Arc`, so a
/// clone is O(1) and shares everything. Writes go through `Arc::make_mut`:
/// the first write to a page that a clone still shares copies the directory,
/// the page's chunk and the page itself, and the clone never changes.
///
/// Only the last page may be shorter than `N`: [`trim`](Self::trim) cuts it
/// to the items in use once a chunk is built, and the first write past its
/// end grows it back to `N` items.
pub(super) struct Pages<T, const N: usize> {
    dir: Arc<[Chunk<T>]>,
}

// Derived `Clone` and `Default` would demand `T: Clone` and `T: Default` for
// the struct, which the `Arc`s do not need.
impl<T, const N: usize> Clone for Pages<T, N> {
    fn clone(&self) -> Self {
        Self {
            dir: Arc::clone(&self.dir),
        }
    }
}

impl<T, const N: usize> Default for Pages<T, N> {
    fn default() -> Self {
        Self { dir: Arc::from([]) }
    }
}

impl<T: Copy + Default, const N: usize> Pages<T, N> {
    /// Item `index`. Panics past the last page.
    #[inline]
    pub fn get(&self, index: u32) -> T {
        let index = index as usize;
        self.page(index / N)[index % N]
    }

    /// `len` items from `start`, which must lie in one page.
    #[inline]
    pub fn slice(&self, start: u32, len: usize) -> &[T] {
        let start = start as usize;
        let offset = start % N;
        &self.page(start / N)[offset..offset + len]
    }

    /// Mutable `len` items from `start`, which must lie in one page. Copies
    /// the directory, the chunk and the page first if a clone shares them,
    /// and grows a trimmed page back to `N` items if the range passes its
    /// end.
    pub fn slice_mut(&mut self, start: u32, len: usize) -> &mut [T] {
        let start = start as usize;
        let (page, offset) = (start / N, start % N);
        let dir = Arc::make_mut(&mut self.dir);
        let chunk = Arc::make_mut(&mut dir[page / CHUNK_PAGES]);
        let page = &mut chunk[page % CHUNK_PAGES];
        if page.len() < offset + len {
            *page = full_page::<T, N>(page);
        }
        &mut Arc::make_mut(page)[offset..offset + len]
    }

    /// Cuts the last page to its first items, so that the pages hold exactly
    /// `len` items, rounded up to [`TRIM_STEP`]. Items past `len` must be
    /// unused: they read as default again if the page grows back. Does
    /// nothing when no page would get shorter.
    pub fn trim(&mut self, len: usize) {
        let Some(last) = self.page_count().checked_sub(1) else {
            return;
        };
        let keep = len
            .saturating_sub(last * N)
            .next_multiple_of(TRIM_STEP)
            .clamp(TRIM_STEP, N);
        let page = &self.dir[last / CHUNK_PAGES][last % CHUNK_PAGES];
        if keep >= page.len() {
            return;
        }
        let short: Page<T> = Arc::from(&page[..keep]);
        let dir = Arc::make_mut(&mut self.dir);
        let chunk = Arc::make_mut(&mut dir[last / CHUNK_PAGES]);
        chunk[last % CHUNK_PAGES] = short;
    }

    /// Items the pages hold, live or free: `N` a page but a trimmed last.
    pub fn held_len(&self) -> usize {
        self.dir.last().map_or(0, |last| {
            let pages = (self.dir.len() - 1) * CHUNK_PAGES + last.len();
            (pages - 1) * N + last.last().map_or(0, |page| page.len())
        })
    }

    /// Appends a page of default items. Rebuilds only the last chunk (or the
    /// directory when a chunk is full), so growing costs O(`CHUNK_PAGES`) per
    /// page, about once per 16 KiB written. A trimmed last page grows back
    /// to `N` items first: only the last page may be short.
    pub fn push_page(&mut self) {
        if let Some(last) = self.page_count().checked_sub(1)
            && self.page(last).len() < N
        {
            let dir = Arc::make_mut(&mut self.dir);
            let chunk = Arc::make_mut(&mut dir[last / CHUNK_PAGES]);
            let page = &mut chunk[last % CHUNK_PAGES];
            *page = full_page::<T, N>(page);
        }
        let page: Page<T> = full_page::<T, N>(&[]);
        if self.dir.last().is_none_or(|last| last.len() == CHUNK_PAGES) {
            let chunk: Chunk<T> = Arc::from([page]);
            self.dir = self.dir.iter().cloned().chain([chunk]).collect();
        } else {
            let dir = Arc::make_mut(&mut self.dir);
            let last = dir.last_mut().expect("the directory has a chunk");
            *last = last.iter().cloned().chain([page]).collect();
        }
    }

    pub fn page_count(&self) -> usize {
        self.dir
            .last()
            .map_or(0, |last| (self.dir.len() - 1) * CHUNK_PAGES + last.len())
    }

    /// Every page in order; page `i` holds items `i * N .. (i + 1) * N`.
    #[cfg(test)]
    pub fn pages(&self) -> impl Iterator<Item = &[T]> {
        self.dir
            .iter()
            .flat_map(|chunk| chunk.iter().map(|page| &page[..]))
    }

    #[inline]
    fn page(&self, page: usize) -> &[T] {
        &self.dir[page / CHUNK_PAGES][page % CHUNK_PAGES]
    }

    /// Reads that remember the last page they touched.
    pub fn reader(&self) -> PageReader<'_, T, N> {
        PageReader {
            pages: self,
            index: usize::MAX,
            page: &[],
        }
    }

    /// Whether page `page` is the same allocation in both. Test support for
    /// checking what copy-on-write copied.
    #[cfg(test)]
    pub fn shares_page(&self, other: &Self, page: usize) -> bool {
        Arc::ptr_eq(
            &self.dir[page / CHUNK_PAGES][page % CHUNK_PAGES],
            &other.dir[page / CHUNK_PAGES][page % CHUNK_PAGES],
        )
    }

    /// Whether chunk `chunk` is the same allocation in both.
    #[cfg(test)]
    pub fn shares_chunk(&self, other: &Self, chunk: usize) -> bool {
        Arc::ptr_eq(&self.dir[chunk], &other.dir[chunk])
    }

    /// Whether both use the same directory allocation: nothing was written
    /// or added between them.
    pub fn shares_list(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.dir, &other.dir)
    }
}

/// A trimmed page keeps a multiple of this many items, so that a chunk's
/// last pages land in a few allocation sizes.
pub(super) const TRIM_STEP: usize = 64;

/// A page of `N` items starting with `items`, the rest default. Built as an
/// array, like every page before trimming, so a new page costs one fill.
fn full_page<T: Copy + Default, const N: usize>(items: &[T]) -> Page<T> {
    let mut page = Arc::new([T::default(); N]);
    if let Some(page) = Arc::get_mut(&mut page) {
        page[..items.len()].copy_from_slice(items);
    }
    page
}

/// Reads from a [`Pages`] that remember the last page touched, so a run of
/// reads in one page goes through the directory and chunk only once. Point
/// lookups walk down a tree whose nodes and blocks were mostly allocated
/// together, so consecutive reads often share a page.
#[derive(Clone)]
pub(super) struct PageReader<'a, T, const N: usize> {
    pages: &'a Pages<T, N>,
    /// The index of `page`; `usize::MAX` before the first read.
    index: usize,
    page: &'a [T],
}

impl<'a, T: Copy + Default, const N: usize> PageReader<'a, T, N> {
    /// Item `index`. Panics past the last page.
    #[inline]
    pub fn get(&mut self, index: u32) -> T {
        let index = index as usize;
        self.page(index / N)[index % N]
    }

    /// `len` items from `start`, which must lie in one page.
    #[inline]
    pub fn slice(&mut self, start: u32, len: usize) -> &'a [T] {
        let start = start as usize;
        let offset = start % N;
        &self.page(start / N)[offset..offset + len]
    }

    /// The items from `start` to the end of its page.
    #[inline]
    pub fn tail(&mut self, start: u32) -> &'a [T] {
        let start = start as usize;
        &self.page(start / N)[start % N..]
    }

    #[inline]
    fn page(&mut self, page: usize) -> &'a [T] {
        if page != self.index {
            self.page = self.pages.page(page);
            self.index = page;
        }
        self.page
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_sees_every_page_and_remembers_the_last() {
        let mut p = pages(CHUNK_PAGES + 2);
        for page in [0, 1, CHUNK_PAGES + 1] {
            p.slice_mut((page * 4) as u32, 4)
                .copy_from_slice(&[page as u32, 1, 2, 3]);
        }
        let mut reader = p.reader();
        assert_eq!(reader.get(4), 1);
        assert_eq!(reader.index, 1);
        assert_eq!(reader.slice(5, 3), [1, 2, 3]);
        assert_eq!(reader.tail(6), [2, 3]);
        assert_eq!(reader.get(0), 0);
        let last = (CHUNK_PAGES + 1) * 4;
        assert_eq!(reader.get(last as u32), (CHUNK_PAGES + 1) as u32);
        assert_eq!(reader.index, CHUNK_PAGES + 1);
    }

    fn pages(count: usize) -> Pages<u32, 4> {
        let mut p = Pages::default();
        for _ in 0..count {
            p.push_page();
        }
        p
    }

    #[test]
    fn trimming_keeps_the_items_in_use_and_a_write_past_them_grows_the_page() {
        let mut p = Pages::<u32, 256>::default();
        p.push_page();
        p.push_page();
        p.slice_mut(256 + 10, 2).copy_from_slice(&[5, 6]);
        assert_eq!(p.held_len(), 512);
        p.trim(256 + 12);
        assert_eq!(p.held_len(), 256 + TRIM_STEP, "rounded up to a step");
        assert_eq!(p.page_count(), 2);
        assert_eq!(p.slice(256 + 10, 2), [5, 6]);
        let snapshot = p.clone();

        // A write inside the kept items stays short; one past them grows it.
        p.slice_mut(256 + 20, 1)[0] = 7;
        assert_eq!(p.held_len(), 256 + TRIM_STEP);
        p.slice_mut(256 + 200, 1)[0] = 8;
        assert_eq!(p.held_len(), 512);
        assert_eq!(p.slice(256 + 10, 2), [5, 6]);
        assert_eq!(
            (p.get(256 + 20), p.get(256 + 200), p.get(256 + 201)),
            (7, 8, 0)
        );
        assert_eq!(snapshot.held_len(), 256 + TRIM_STEP);
        assert_eq!(snapshot.get(256 + 20), 0, "the snapshot never changes");

        // A new page first grows a trimmed last page back.
        p.trim(256 + 1);
        p.push_page();
        assert_eq!(p.held_len(), 3 * 256);
        assert!(p.pages().all(|page| page.len() == 256));
    }

    #[test]
    fn trimming_an_empty_or_full_page_does_nothing() {
        let mut p = Pages::<u32, 256>::default();
        p.trim(0);
        assert_eq!(p.held_len(), 0);
        p.push_page();
        p.trim(256);
        assert_eq!(p.held_len(), 256);
        p.trim(0);
        assert_eq!(p.held_len(), TRIM_STEP, "a page keeps at least one step");
    }

    #[test]
    fn items_are_addressed_across_pages() {
        let mut p = pages(3);
        p.slice_mut(5, 2).copy_from_slice(&[10, 11]);
        assert_eq!(p.get(5), 10);
        assert_eq!(p.get(6), 11);
        assert_eq!(p.slice(4, 4), [0, 10, 11, 0]);
        assert_eq!(p.page_count(), 3);
    }

    #[test]
    fn items_are_addressed_across_chunks() {
        let mut p = pages(CHUNK_PAGES * 2 + 3);
        assert_eq!(p.page_count(), CHUNK_PAGES * 2 + 3);
        assert_eq!(p.dir.len(), 3);
        for page in [0, CHUNK_PAGES - 1, CHUNK_PAGES, CHUNK_PAGES * 2 + 2] {
            let at = (page * 4 + 1) as u32;
            p.slice_mut(at, 1)[0] = page as u32 + 1;
        }
        for page in [0, CHUNK_PAGES - 1, CHUNK_PAGES, CHUNK_PAGES * 2 + 2] {
            assert_eq!(p.get((page * 4 + 1) as u32), page as u32 + 1);
        }
        let listed: Vec<u32> = p.pages().map(|page| page[1]).collect();
        assert_eq!(listed.len(), p.page_count());
        assert_eq!(listed[CHUNK_PAGES], CHUNK_PAGES as u32 + 1);
        assert_eq!(listed[1], 0);
    }

    #[test]
    fn a_write_copies_only_the_shared_page_and_chunk_it_touches() {
        let mut live = pages(CHUNK_PAGES + 2);
        live.slice_mut(0, 1)[0] = 1;
        let snapshot = live.clone();
        assert!(live.shares_list(&snapshot));

        // A page in the second chunk.
        let page = CHUNK_PAGES + 1;
        live.slice_mut((page * 4 + 1) as u32, 1)[0] = 2;
        assert!(!live.shares_list(&snapshot), "the directory is copied");
        assert!(
            live.shares_chunk(&snapshot, 0),
            "the untouched chunk is not"
        );
        assert!(!live.shares_chunk(&snapshot, 1));
        assert!(live.shares_page(&snapshot, 0));
        assert!(live.shares_page(&snapshot, CHUNK_PAGES));
        assert!(!live.shares_page(&snapshot, page));
        assert_eq!(snapshot.get((page * 4 + 1) as u32), 0);
        assert_eq!(live.get((page * 4 + 1) as u32), 2);
        assert_eq!(live.get(0), 1);
    }

    #[test]
    fn an_unshared_page_is_written_in_place() {
        let mut live = pages(1);
        let before = Arc::as_ptr(&live.dir[0][0]).cast::<u32>();
        live.slice_mut(1, 1)[0] = 7;
        assert_eq!(Arc::as_ptr(&live.dir[0][0]).cast::<u32>(), before);
    }

    #[test]
    fn growing_never_changes_a_clone() {
        let mut live = pages(CHUNK_PAGES);
        live.slice_mut(3, 1)[0] = 9;
        let snapshot = live.clone();
        live.push_page();
        live.push_page();
        assert_eq!(snapshot.page_count(), CHUNK_PAGES);
        assert_eq!(live.page_count(), CHUNK_PAGES + 2);
        assert!(
            live.shares_chunk(&snapshot, 0),
            "a full chunk is not rebuilt"
        );
        assert!(live.shares_page(&snapshot, 0));
        live.slice_mut(3, 1)[0] = 4;
        assert_eq!(snapshot.get(3), 9);
    }
}
