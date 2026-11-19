//! Copy-on-write paged storage.

use std::sync::Arc;

/// A growable array of `T` in fixed pages of `N` items. Each page is behind
/// an `Arc`, and so is the page list, so a clone is O(1) and shares every
/// page. Writes go through `Arc::make_mut`: the first write to a page (or the
/// list) that a clone still shares copies it, and the clone never changes.
#[derive(Clone, Default)]
pub(super) struct Pages<T, const N: usize> {
    list: Arc<Vec<Arc<[T; N]>>>,
}

impl<T: Copy + Default, const N: usize> Pages<T, N> {
    /// Item `index`. Panics past the last page.
    pub fn get(&self, index: u32) -> T {
        let index = index as usize;
        self.list[index / N][index % N]
    }

    /// `len` items from `start`, which must lie in one page.
    pub fn slice(&self, start: u32, len: usize) -> &[T] {
        let start = start as usize;
        let offset = start % N;
        &self.list[start / N][offset..offset + len]
    }

    /// Mutable `len` items from `start`, which must lie in one page. Copies
    /// the page list and that page first if a clone shares them.
    pub fn slice_mut(&mut self, start: u32, len: usize) -> &mut [T] {
        let start = start as usize;
        let offset = start % N;
        let list = Arc::make_mut(&mut self.list);
        &mut Arc::make_mut(&mut list[start / N])[offset..offset + len]
    }

    /// Appends a page of default items.
    pub fn push_page(&mut self) {
        Arc::make_mut(&mut self.list).push(Arc::new([T::default(); N]));
    }

    pub fn page_count(&self) -> usize {
        self.list.len()
    }

    /// Whether page `page` is the same allocation in both. Test support for
    /// checking what copy-on-write copied.
    #[cfg(test)]
    pub fn shares_page(&self, other: &Self, page: usize) -> bool {
        Arc::ptr_eq(&self.list[page], &other.list[page])
    }

    /// Whether both use the same page list allocation.
    #[cfg(test)]
    pub fn shares_list(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.list, &other.list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(count: usize) -> Pages<u32, 4> {
        let mut p = Pages::default();
        for _ in 0..count {
            p.push_page();
        }
        p
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
    fn a_write_copies_only_the_shared_page_it_touches() {
        let mut live = pages(3);
        live.slice_mut(0, 1)[0] = 1;
        let snapshot = live.clone();
        assert!(live.shares_list(&snapshot));

        live.slice_mut(9, 1)[0] = 2;
        assert!(!live.shares_list(&snapshot));
        assert!(live.shares_page(&snapshot, 0));
        assert!(live.shares_page(&snapshot, 1));
        assert!(!live.shares_page(&snapshot, 2));
        assert_eq!(snapshot.get(9), 0);
        assert_eq!(live.get(9), 2);
    }

    #[test]
    fn an_unshared_page_is_written_in_place() {
        let mut live = pages(1);
        let before = Arc::as_ptr(&live.list[0]);
        live.slice_mut(1, 1)[0] = 7;
        assert_eq!(Arc::as_ptr(&live.list[0]), before);
    }

    #[test]
    fn growing_never_changes_a_clone() {
        let mut live = pages(1);
        let snapshot = live.clone();
        live.push_page();
        assert_eq!(snapshot.page_count(), 1);
        assert_eq!(live.page_count(), 2);
    }
}
