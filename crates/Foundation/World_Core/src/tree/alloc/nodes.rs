//! Node slot allocation: a LIFO free list over a growing slot range.

use super::pages::Pages;
use crate::tree::node::{MAX_NODES, Node};

/// Which node slots are in use. The node contents live in the pages.
#[derive(Default)]
pub(super) struct NodeSlots {
    /// One past the highest slot ever handed out.
    end: u32,
    /// Freed slots; the most recently freed is reused first.
    free: Vec<u32>,
    live: usize,
}

impl NodeSlots {
    /// A slot for a new node: the most recently freed one, else the next
    /// unused one, adding a page when the slot starts a new one. Panics when
    /// node indices would reach the uniform bit of a child word.
    pub fn alloc<const N: usize>(&mut self, pages: &mut Pages<Node, N>) -> u32 {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                let index = self.end;
                assert!(
                    index < MAX_NODES,
                    "the node arena is full ({MAX_NODES} nodes)"
                );
                if index as usize == pages.page_count() * N {
                    pages.push_page();
                }
                self.end += 1;
                index
            }
        };
        self.live += 1;
        index
    }

    /// Returns a slot for reuse. Its page is not written.
    pub fn free(&mut self, index: u32) {
        debug_assert!(index < self.end, "slot {index} was never allocated");
        self.live -= 1;
        self.free.push(index);
    }

    pub fn live(&self) -> usize {
        self.live
    }

    pub fn free_count(&self) -> usize {
        self.free.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freed_slots_are_reused_last_in_first_out() {
        let mut pages = Pages::<Node, 4>::default();
        let mut slots = NodeSlots::default();
        let ids: Vec<u32> = (0..3).map(|_| slots.alloc(&mut pages)).collect();
        assert_eq!(ids, [0, 1, 2]);
        slots.free(0);
        slots.free(2);
        assert_eq!(slots.free_count(), 2);
        assert_eq!(slots.alloc(&mut pages), 2);
        assert_eq!(slots.alloc(&mut pages), 0);
        assert_eq!(slots.alloc(&mut pages), 3);
        assert_eq!(slots.live(), 4);
    }

    #[test]
    fn pages_are_added_only_when_a_slot_needs_one() {
        let mut pages = Pages::<Node, 4>::default();
        let mut slots = NodeSlots::default();
        for expected_pages in [1, 1, 1, 1, 2] {
            slots.alloc(&mut pages);
            assert_eq!(pages.page_count(), expected_pages);
        }
    }
}
