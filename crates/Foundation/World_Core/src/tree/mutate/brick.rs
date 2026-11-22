//! Level-1 nodes: reading, rewriting and packing their 64 leaf records, and
//! the palette blocks those records own.

use super::Writer;
use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;
use crate::tree::node::leaf::{self, PackedLeaf, Record, Voxels};
use crate::tree::node::{
    BRICK_HEADER_MAX_WORDS, BrickHeader, Child, MAX_BLOCK_LEN, Node, PAIR_WORDS, Packed,
    PackedNode, RECORD_WORDS, UNIFORM_BIT, bit, slots,
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
        let header = BrickHeader::read(block, node.info);
        let mut records = [Record::AIR; TREE_FANOUT as usize];
        let mut half = 2 * header.len();
        let mut at = header.leaves_start();
        for slot in slots(node.mask) {
            let b = bit(slot);
            records[slot as usize] = if header.default_full() & b != 0 {
                Record::filled(header.default)
            } else if header.full & b != 0 {
                half += 1;
                let word = block[(half - 1) / 2];
                Record::filled(MaterialId((word >> (16 * ((half - 1) % 2))) as u16))
            } else if header.defaults & b != 0 {
                at += 2;
                let mask = u64::from(block[at - 2]) | u64::from(block[at - 1]) << 32;
                Record::single(mask, header.default)
            } else if header.pairs & b != 0 {
                at += PAIR_WORDS;
                Record::read_pair(&block[at - PAIR_WORDS..at])
            } else {
                at += RECORD_WORDS;
                Record::read(&block[at - RECORD_WORDS..at])
            };
        }
        records
    }

    /// Leaf `slot` of a level-1 child whose node, if it is one, is `node`.
    pub(super) fn record(&self, child: Child, node: Option<Node>, slot: u32) -> Record {
        match (child, node) {
            (_, Some(node)) if node.mask & bit(slot) == 0 => Record::AIR,
            (_, Some(node)) => {
                let block = self.store().block(node.base, node.block_len(1));
                BrickHeader::read(block, node.info).leaf(block, node.mask, slot)
            }
            (Child::Uniform(m), None) => Record::filled(m),
            _ => Record::AIR,
        }
    }

    /// The header of a stored level-1 node.
    pub(super) fn header(&self, node: Node) -> BrickHeader {
        let len = node.block_len(1).min(BRICK_HEADER_MAX_WORDS);
        BrickHeader::read(self.store().block(node.base, len), node.info)
    }

    /// Overwrites leaf `slot` of a stored level-1 node with `new` where that
    /// keeps the node canonical without repacking: the slot holds a partial
    /// leaf and `new` is a partial leaf stored the same way. A default
    /// partial leaf (a leaf mask) takes another default partial leaf, a
    /// two-material leaf another two-material leaf, and a record any other
    /// partial leaf but one of the default material, which would join the
    /// default mask. Full single-material leaves decide the default and the
    /// masks, and only they can make the node collapse, so those always
    /// repack; a full leaf of several materials is an ordinary record (or a
    /// pair). Returns whether it wrote.
    pub(super) fn patch_record(&mut self, node: Node, slot: u32, new: Record) -> bool {
        if new.mask == 0 || new.uniform().is_some() || node.mask & bit(slot) == 0 {
            return false;
        }
        let header = self.header(node);
        let b = bit(slot);
        if header.full & b != 0 {
            return false;
        }
        let default_partial = header.defaults & b != 0;
        if default_partial != (new.material() == Some(header.default)) {
            return false;
        }
        if (header.pairs & b != 0) != new.is_pair() {
            return false;
        }
        let at = node.base + header.record_offset(node.mask, slot) as u32;
        if new.is_pair() {
            self.store_mut().write_block(at, &new.pair_words());
            return true;
        }
        let words = new.words();
        let len = if default_partial { 2 } else { RECORD_WORDS };
        self.store_mut().write_block(at, &words[..len]);
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
            // Two materials: stored inline in the brick (decision 0031).
            PackedLeaf::Palette { mask, block } if leaf::is_pair(block.words()[0]) => {
                self.free_palette(old);
                Record::pair(mask, block.words()[0], block.pair_indices())
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
                Record::palette(mask, base)
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
/// mask, a header naming the default material (the most common among the
/// full leaves, ties to the lower id) and its leaves, and the other non-air
/// leaves in slot order (layout in `node`).
pub(super) fn pack(records: &Records) -> Packed<MAX_BLOCK_LEN> {
    // One pass for three masks: the non-air leaves, the full one-material
    // leaves and the partial one-material leaves. Little else is live in
    // it, so it stays in registers; the rest reads the set bits only.
    let mut mask = 0u64;
    let mut full = 0u64;
    let mut singles = 0u64;
    let mut pairs = 0u64;
    for (slot, r) in (0..).zip(records) {
        if r.mask == 0 {
            continue;
        }
        mask |= bit(slot);
        if r.indices != 0 {
            pairs |= bit(slot);
        } else if r.payload & UNIFORM_BIT != 0 {
            if r.mask == u64::MAX {
                full |= bit(slot);
            } else {
                singles |= bit(slot);
            }
        }
    }
    if mask == 0 {
        return Packed::Filled(MaterialId::AIR);
    }
    // Usually the full leaves are one material, which is then the default.
    let (default, default_full) = if full == 0 {
        (MaterialId::AIR, 0)
    } else {
        let first = records[full.trailing_zeros() as usize].payload;
        let of_first = of_payload(records, first, full);
        if of_first == full {
            if full == u64::MAX {
                return Packed::Filled(MaterialId(first as u16));
            }
            (MaterialId(first as u16), full)
        } else {
            // Full leaves of several materials: the most common is the
            // default.
            let default = most_common_full(records);
            let word = UNIFORM_BIT | u32::from(default.0);
            (default, of_payload(records, word, full))
        }
    };
    // Partial leaves of the default join the default mask (decision 0025).
    let partial = if default_full == 0 {
        0
    } else {
        of_payload(records, UNIFORM_BIT | u32::from(default.0), singles)
    };
    let header = BrickHeader {
        defaults: default_full | partial,
        default,
        full,
        pairs,
    };
    let mut words = [0; MAX_BLOCK_LEN];
    let head = header.len();
    words[..head].copy_from_slice(&header.words()[..head]);
    // Other full leaves: their materials as halves, two a word (decision
    // 0033).
    for (k, slot) in slots(header.other_full()).enumerate() {
        words[head + k / 2] |= (records[slot as usize].payload & 0xFFFF) << (16 * (k % 2));
    }
    let mut len = header.leaves_start();
    for slot in slots(mask & !full) {
        let r = records[slot as usize];
        let b = bit(slot);
        if partial & b != 0 {
            words[len] = r.mask as u32;
            words[len + 1] = (r.mask >> 32) as u32;
            len += 2;
        } else if pairs & b != 0 {
            words[len..len + PAIR_WORDS].copy_from_slice(&r.pair_words());
            len += PAIR_WORDS;
        } else {
            words[len..len + RECORD_WORDS].copy_from_slice(&r.words());
            len += RECORD_WORDS;
        }
    }
    Packed::Node(PackedNode::from_array(mask, header.info(len), words, len))
}

/// The leaves among `among` whose payload is `payload`.
fn of_payload(records: &Records, payload: u32, among: u64) -> u64 {
    slots(among)
        .filter(|&slot| records[slot as usize].payload == payload)
        .fold(0, |m, slot| m | bit(slot))
}

/// The most common material among the full leaves (at least one), ties to
/// the lower id.
fn most_common_full(records: &Records) -> MaterialId {
    let mut counts: [(MaterialId, u32); TREE_FANOUT as usize] =
        [(MaterialId::AIR, 0); TREE_FANOUT as usize];
    let mut distinct = 0;
    for m in records.iter().filter_map(|r| r.uniform()) {
        match counts[..distinct].iter_mut().find(|(c, _)| *c == m) {
            Some(entry) => entry.1 += 1,
            None => {
                counts[distinct] = (m, 1);
                distinct += 1;
            }
        }
    }
    counts[..distinct]
        .iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map_or(MaterialId::AIR, |&(m, _)| m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::node::BRICK_FULL_MASK;

    fn header_of(records: &Records) -> BrickHeader {
        let Packed::Node(p) = pack(records) else {
            panic!("mixed leaves need a node");
        };
        BrickHeader::read(p.words(), p.info)
    }

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
        records[63] = Record::palette(1 << 40, 1234);
        let Packed::Node(p) = pack(&records) else {
            panic!("mixed leaves need a node");
        };
        assert_eq!(p.mask, bit(3) | bit(40) | bit(63));
        // The full leaf of 5 is the default: in the header, with no record.
        assert_eq!(p.info, 9, "a header and two records");
        assert_eq!(
            p.words(),
            [0, 1 << 8, 5, 0b1010, 0, UNIFORM_BIT | 9, 0, 1 << 8, 1234,]
        );
    }

    #[test]
    fn a_full_single_material_leaf_among_others_does_not_collapse() {
        let stone = Record::filled(MaterialId(2));
        let mut records = [stone; 64];
        records[7] = Record::single(!1, MaterialId(2));
        // A default partial leaf: the full mask, then its leaf mask alone.
        let Packed::Node(p) = pack(&records) else {
            panic!("a partial leaf needs a node");
        };
        assert_eq!((p.mask, p.info), (u64::MAX, 7 | BRICK_FULL_MASK));
        assert_eq!(
            p.words(),
            [u32::MAX, u32::MAX, 2, !(1 << 7), u32::MAX, !1, u32::MAX]
        );
        // A second full material is stored as a half word after a header
        // with the full mask; the more common one is default.
        records[7] = Record::filled(MaterialId(3));
        let Packed::Node(p) = pack(&records) else {
            panic!("two full materials need a node");
        };
        assert_eq!(p.info, 6 | BRICK_FULL_MASK);
        assert_eq!(p.words(), [!(1 << 7), u32::MAX, 2, u32::MAX, u32::MAX, 3]);
        // Three of them share two words, the last high half zero.
        records[9] = Record::filled(MaterialId(5));
        records[60] = Record::filled(MaterialId(4));
        let Packed::Node(p) = pack(&records) else {
            panic!("full materials need a node");
        };
        assert_eq!(p.words()[5..], [3 | 5 << 16, 4]);
        let h = header_of(&records);
        assert_eq!(
            [7, 9, 60].map(|s| h.leaf(p.words(), p.mask, s)),
            [3, 5, 4].map(|m| Record::filled(MaterialId(m)))
        );
    }

    #[test]
    fn default_partial_leaves_store_their_mask_alone() {
        let mut records = [Record::AIR; 64];
        records[0] = Record::filled(MaterialId(4));
        records[2] = Record::single(0b11, MaterialId(4));
        records[5] = Record::single(0b101, MaterialId(6));
        records[9] = Record::single(1 << 40, MaterialId(4));
        records[63] = Record::single(1 << 63, MaterialId(4));
        let Packed::Node(p) = pack(&records) else {
            panic!("mixed leaves need a node");
        };
        assert_eq!(p.info, (5 + 2 + 3 + 2 + 2) | BRICK_FULL_MASK);
        let defaults = bit(0) | bit(2) | bit(9) | bit(63);
        assert_eq!(
            p.words(),
            [
                defaults as u32,
                (defaults >> 32) as u32,
                4,
                1,
                0,
                0b11,
                0,
                0b101,
                0,
                UNIFORM_BIT | 6,
                0,
                1 << 8,
                0,
                1 << 31,
            ]
        );
        // Without the partial leaves of 4, the header is short again.
        for slot in [2, 9, 63] {
            records[slot] = Record::AIR;
        }
        let h = header_of(&records);
        assert_eq!((h.len(), h.defaults, h.full), (3, bit(0), bit(0)));
    }

    #[test]
    fn two_material_leaves_pack_inline_after_a_pair_mask() {
        let mut records = [Record::AIR; 64];
        records[0] = Record::filled(MaterialId(4));
        records[3] = Record::pair(u64::MAX, 4 | 6 << 16, 0b110);
        records[8] = Record::single(0b1, MaterialId(9));
        let Packed::Node(p) = pack(&records) else {
            panic!("mixed leaves need a node");
        };
        assert_eq!(p.info, (5 + 5 + 3) | crate::tree::node::BRICK_PAIR_MASK);
        assert_eq!(
            p.words(),
            [
                1,
                0,
                4,
                bit(3) as u32,
                0,
                u32::MAX,
                u32::MAX,
                4 | 6 << 16,
                0b110,
                0,
                1,
                0,
                UNIFORM_BIT | 9,
            ]
        );
        let h = header_of(&records);
        assert_eq!(h.leaf(p.words(), p.mask, 3), records[3]);
        assert_eq!(h.leaf(p.words(), p.mask, 8), records[8]);
    }

    #[test]
    fn without_full_leaves_there_is_no_default() {
        let mut records = [Record::AIR; 64];
        records[1] = Record::single(0b1, MaterialId(9));
        records[3] = Record::single(0b100, MaterialId(9));
        let Packed::Node(p) = pack(&records) else {
            panic!("partial leaves need a node");
        };
        assert_eq!(p.info, 3 + 2 * RECORD_WORDS as u32);
        assert_eq!(BrickHeader::read(p.words(), p.info), BrickHeader::default());
    }

    #[test]
    fn the_most_common_full_material_wins_ties_to_the_lower_id() {
        let mut records = [Record::AIR; 64];
        for slot in [1, 2, 9] {
            records[slot] = Record::filled(MaterialId(7));
        }
        for slot in [4, 5, 6] {
            records[slot] = Record::filled(MaterialId(3));
        }
        records[8] = Record::single(1, MaterialId(3));
        // Partial leaves of 7 do not count.
        records[10] = Record::single(1, MaterialId(7));
        records[11] = Record::single(1, MaterialId(7));
        let h = header_of(&records);
        assert_eq!(h.default, MaterialId(3), "a tie goes to the lower id");
        assert_eq!(h.defaults, bit(4) | bit(5) | bit(6) | bit(8));
        assert_eq!(h.full, bit(1) | bit(2) | bit(4) | bit(5) | bit(6) | bit(9));
        records[12] = Record::filled(MaterialId(7));
        assert_eq!(header_of(&records).default, MaterialId(7));
        assert_eq!(most_common_full(&records), MaterialId(7));
    }
}
