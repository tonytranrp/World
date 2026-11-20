//! Reading and packing the 64 child words of a level-2-or-higher node.

use super::Writer;
use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;
use crate::tree::node::{Child, Node, Packed, PackedNode, bit, rank, slots};

/// The children of one node in slot order, air included.
pub(super) type Children = [Child; TREE_FANOUT as usize];

impl Writer<'_> {
    /// The 64 children of a child at level 2 or higher: a node's decoded
    /// words, or 64 copies of an air or uniform child (what splitting it
    /// gives).
    pub(super) fn children(&self, child: Child) -> Children {
        let Child::Node(index) = child else {
            return [child; TREE_FANOUT as usize];
        };
        let node = self.store().node(index);
        let block = self
            .store()
            .block(node.base, node.mask.count_ones() as usize);
        let mut children = [Child::Air; TREE_FANOUT as usize];
        for (slot, &word) in slots(node.mask).zip(block) {
            children[slot as usize] = Child::from_word(word);
        }
        children
    }

    /// Child `slot` of a stored node at level 2 or higher.
    #[inline]
    pub(super) fn child(&self, node: Node, slot: u32) -> Child {
        if node.mask & bit(slot) == 0 {
            return Child::Air;
        }
        Child::from_word(self.store().word(node.base + rank(node.mask, slot) as u32))
    }

    /// Overwrites child `slot` of a stored node at level 2 or higher with
    /// `new` where that keeps the node canonical without repacking: the slot
    /// already holds a child, `new` is not air, and the node cannot collapse
    /// (it can only once every slot holds a uniform child). Returns whether
    /// it wrote.
    pub(super) fn patch_child(&mut self, node: Node, slot: u32, new: Child) -> bool {
        let Some(word) = new.word() else {
            return false;
        };
        let may_collapse = node.mask == u64::MAX && matches!(new, Child::Uniform(_));
        if node.mask & bit(slot) == 0 || may_collapse {
            return false;
        }
        let at = node.base + rank(node.mask, slot) as u32;
        self.store_mut().write_block(at, &[word]);
        true
    }
}

/// Packs 64 canonical children into canonical node content: all air or all
/// the same uniform child collapses to [`Packed::Filled`]; anything else is a
/// mask and one word per non-air child.
pub(super) fn pack(children: &Children) -> Packed<{ TREE_FANOUT as usize }> {
    let mut mask = 0;
    let mut words = [0; TREE_FANOUT as usize];
    let mut len = 0;
    for (slot, child) in (0..).zip(children) {
        if let Some(word) = child.word() {
            mask |= bit(slot);
            words[len] = word;
            len += 1;
        }
    }
    if mask == 0 {
        return Packed::Filled(MaterialId::AIR);
    }
    if children.iter().all(|&c| c == children[0]) {
        match children[0] {
            Child::Uniform(material) => return Packed::Filled(material),
            Child::NotResident => return Packed::NotResident,
            Child::Air | Child::Node(_) => {}
        }
    }
    Packed::Node(PackedNode::from_array(mask, 0, words, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_air_and_all_one_material_collapse() {
        assert_eq!(pack(&[Child::Air; 64]), Packed::Filled(MaterialId::AIR));
        let stone = Child::Uniform(MaterialId(2));
        assert_eq!(pack(&[stone; 64]), Packed::Filled(MaterialId(2)));
    }

    #[test]
    fn mixed_children_pack_in_slot_order() {
        let mut children = [Child::Air; 64];
        children[3] = Child::Node(9);
        children[40] = Child::Uniform(MaterialId(5));
        let Packed::Node(p) = pack(&children) else {
            panic!("mixed children need a node");
        };
        assert_eq!(p.mask, bit(3) | bit(40));
        assert_eq!(p.words(), [9, (1 << 31) | 5]);
    }

    #[test]
    fn one_different_or_missing_child_prevents_collapse() {
        let stone = Child::Uniform(MaterialId(2));
        let mut children = [stone; 64];
        children[63] = Child::Uniform(MaterialId(3));
        assert!(matches!(pack(&children), Packed::Node(p) if p.mask == u64::MAX));
        children[63] = Child::Air;
        assert!(matches!(pack(&children), Packed::Node(p) if p.mask == !bit(63)));
    }

    #[test]
    fn all_not_resident_collapses_and_mixed_stays() {
        assert_eq!(pack(&[Child::NotResident; 64]), Packed::NotResident);
        let mut children = [Child::NotResident; 64];
        children[0] = Child::Uniform(MaterialId(2));
        let Packed::Node(p) = pack(&children) else {
            panic!("mixed residency needs a node");
        };
        assert_eq!(p.mask, u64::MAX);
        assert_eq!(p.words()[1], crate::tree::node::NOT_RESIDENT);
    }
}
