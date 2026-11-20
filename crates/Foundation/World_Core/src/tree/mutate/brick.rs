//! Level-1 nodes: reading, rewriting and packing their 64 leaf records, and
//! the palette blocks those records own.

use super::Writer;
use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;
use crate::tree::node::leaf::{self, PackedLeaf, Record, Voxels};
use crate::tree::node::{
    Child, MAX_BLOCK_LEN, Node, Packed, PackedNode, RECORD_WORDS, bit, rank, slots,
};

/// The leaves of one level-1 node in slot order, air included.
pub(super) type Records = [Record; TREE_FANOUT as usize];

impl Writer<'_> {
    /// The 64 leaf records of a level-1 child: a node's stored records, or 64
    /// copies of an air or uniform leaf (what splitting it gives).
    pub(super) fn records(&self, child: Child) -> Records {
        let index = match child {
            Child::Air => return [Record::AIR; TREE_FANOUT as usize],
            Child::Uniform(m) => return [Record::filled(m); TREE_FANOUT as usize],
            Child::Node(index) => index,
            // Only children at the chunk level and above can be unloaded.
            Child::NotResident => unreachable!("level-1 children are always resident"),
        };
        let node = self.store().node(index);
        let block = self.store().block(node.base, node.block_len(1));
        let mut records = [Record::AIR; TREE_FANOUT as usize];
        for (slot, words) in slots(node.mask).zip(block.chunks_exact(RECORD_WORDS)) {
            records[slot as usize] = Record::read(words);
        }
        records
    }

    /// Leaf `slot` of a level-1 child whose node, if it is one, is `node`.
    pub(super) fn record(&self, child: Child, node: Option<Node>, slot: u32) -> Record {
        match (child, node) {
            (_, Some(node)) if node.mask & bit(slot) == 0 => Record::AIR,
            (_, Some(node)) => {
                let at = node.base + (RECORD_WORDS * rank(node.mask, slot)) as u32;
                Record::read(self.store().block(at, RECORD_WORDS))
            }
            (Child::Uniform(m), None) => Record::filled(m),
            _ => Record::AIR,
        }
    }

    /// Overwrites leaf `slot` of a stored level-1 node with `new` where that
    /// keeps the node canonical without repacking: the slot already holds a
    /// leaf, `new` is not air, and the node cannot collapse (it can only once
    /// every slot holds a uniform leaf). Returns whether it wrote.
    pub(super) fn patch_record(&mut self, node: Node, slot: u32, new: Record) -> bool {
        let may_collapse = node.mask == u64::MAX && new.uniform().is_some();
        if new.mask == 0 || node.mask & bit(slot) == 0 || may_collapse {
            return false;
        }
        let at = node.base + (RECORD_WORDS * rank(node.mask, slot)) as u32;
        self.store_mut().write_block(at, &new.words());
        true
    }

    /// The 64 voxels of a stored leaf.
    pub(super) fn leaf_voxels(&self, record: Record) -> Voxels {
        match record.palette_base() {
            Some(base) => leaf::decode(record, self.palette_block(record, base)),
            None => leaf::decode(record, &[]),
        }
    }

    /// Stores `new` in place of the leaf `old` and returns its record. A
    /// palette block of the same length is overwritten in place where it
    /// differs; otherwise the old one is freed and a new one allocated.
    pub(super) fn store_leaf(&mut self, old: Record, new: PackedLeaf) -> Record {
        let record = match new {
            PackedLeaf::Inline(record) => {
                self.free_palette(old);
                record
            }
            PackedLeaf::Palette { mask, block } => {
                let words = block.words();
                let base = match old.palette_base() {
                    Some(base) => {
                        let old_len = self.palette_block(old, base).len();
                        if old_len == words.len() {
                            if self.store().block(base, old_len) != words {
                                self.store_mut().write_block(base, words);
                            }
                            base
                        } else {
                            self.store_mut().free_block(base, old_len);
                            self.store_mut().alloc_block(words)
                        }
                    }
                    None => self.store_mut().alloc_block(words),
                };
                Record {
                    mask,
                    payload: base,
                }
            }
        };
        self.store_mut().count_leaf(old.is_leaf(), record.is_leaf());
        record
    }

    /// Frees what a leaf owns, when its whole level-1 node goes.
    pub(super) fn free_leaf(&mut self, record: Record) {
        self.free_palette(record);
        self.store_mut().count_leaf(record.is_leaf(), false);
    }

    fn free_palette(&mut self, record: Record) {
        if let Some(base) = record.palette_base() {
            let len = self.palette_block(record, base).len();
            self.store_mut().free_block(base, len);
        }
    }

    /// The palette block of `record`, stored at `base`.
    fn palette_block(&self, record: Record, base: u32) -> &[u32] {
        let len = leaf::block_len(record.mask, self.store().word(base));
        self.store().block(base, len)
    }
}

/// Packs 64 canonical leaves into canonical level-1 content: all air or all
/// the same uniform leaf collapses to [`Packed::Filled`]; anything else is a
/// mask and one record per non-air leaf.
pub(super) fn pack(records: &Records) -> Packed<MAX_BLOCK_LEN> {
    let mut mask = 0;
    let mut words = [0; MAX_BLOCK_LEN];
    let mut len = 0;
    for (slot, record) in (0..).zip(records) {
        if record.mask != 0 {
            mask |= bit(slot);
            words[len..len + RECORD_WORDS].copy_from_slice(&record.words());
            len += RECORD_WORDS;
        }
    }
    if mask == 0 {
        return Packed::Filled(MaterialId::AIR);
    }
    if let Some(material) = records[0].uniform()
        && records.iter().all(|&r| r == records[0])
    {
        return Packed::Filled(material);
    }
    Packed::Node(PackedNode::new(mask, &words[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::node::UNIFORM_BIT;

    #[test]
    fn all_air_and_all_one_uniform_leaf_collapse() {
        assert_eq!(pack(&[Record::AIR; 64]), Packed::Filled(MaterialId::AIR));
        let stone = Record::filled(MaterialId(2));
        assert_eq!(pack(&[stone; 64]), Packed::Filled(MaterialId(2)));
    }

    #[test]
    fn leaves_pack_as_three_word_records_in_slot_order() {
        let mut records = [Record::AIR; 64];
        records[3] = Record::single(0b1010, MaterialId(9));
        records[40] = Record::filled(MaterialId(5));
        records[63] = Record {
            mask: 1 << 40,
            payload: 1234,
        };
        let Packed::Node(p) = pack(&records) else {
            panic!("mixed leaves need a node");
        };
        assert_eq!(p.mask, bit(3) | bit(40) | bit(63));
        assert_eq!(
            p.words(),
            [
                0b1010,
                0,
                UNIFORM_BIT | 9,
                u32::MAX,
                u32::MAX,
                UNIFORM_BIT | 5,
                0,
                1 << 8,
                1234,
            ]
        );
    }

    #[test]
    fn a_full_single_material_leaf_among_others_does_not_collapse() {
        let stone = Record::filled(MaterialId(2));
        let mut records = [stone; 64];
        records[7] = Record::single(!1, MaterialId(2));
        assert!(matches!(pack(&records), Packed::Node(p) if p.mask == u64::MAX));
        records[7] = Record::filled(MaterialId(3));
        assert!(matches!(pack(&records), Packed::Node(p) if p.words().len() == 192));
    }
}
