//! Leaves: the 4x4x4-voxel children of a level-1 node, stored as inline
//! records with an optional palette block (layout in the module above).

use super::{RECORD_WORDS, UNIFORM_BIT, slots};
use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;

/// The 64 voxels of a leaf in slot order, air included.
pub(crate) type Voxels = [MaterialId; TREE_FANOUT as usize];

/// The longest palette: every voxel a different material.
const MAX_PALETTE: usize = TREE_FANOUT as usize;

/// The longest palette block: header, a full palette and 64 eight-bit indices.
pub(crate) const MAX_PALETTE_BLOCK: usize = 1 + palette_words(MAX_PALETTE) + index_words(64, 8);

const _: () = assert!(MAX_PALETTE_BLOCK == 49);
const _: () = assert!(MAX_PALETTE_BLOCK <= super::MAX_BLOCK_LEN);

/// One leaf as its level-1 parent stores it: the record's mask and payload.
/// A zero mask is an air leaf, which has no record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Record {
    /// Bit `j` is set when voxel `j` is solid.
    pub mask: u64,
    /// [`UNIFORM_BIT`] and a material, or the offset of a palette block.
    pub payload: u32,
}

impl Record {
    pub const AIR: Self = Self {
        mask: 0,
        payload: 0,
    };

    /// The leaf whose every voxel is `material`.
    pub const fn filled(material: MaterialId) -> Self {
        if material.is_air() {
            Self::AIR
        } else {
            Self::single(u64::MAX, material)
        }
    }

    /// Solid voxels `mask`, all of `material`.
    pub const fn single(mask: u64, material: MaterialId) -> Self {
        Self {
            mask,
            payload: UNIFORM_BIT | material.0 as u32,
        }
    }

    /// Reads a stored record from its three words.
    pub fn read(words: &[u32]) -> Self {
        Self {
            mask: u64::from(words[0]) | u64::from(words[1]) << 32,
            payload: words[2],
        }
    }

    /// The three stored words.
    pub const fn words(self) -> [u32; RECORD_WORDS] {
        [self.mask as u32, (self.mask >> 32) as u32, self.payload]
    }

    /// The one material of every solid voxel, when there is no palette.
    pub const fn material(self) -> Option<MaterialId> {
        if self.payload & UNIFORM_BIT != 0 {
            Some(MaterialId(self.payload as u16))
        } else {
            None
        }
    }

    /// The offset of the palette block, when the leaf has one.
    pub const fn palette_base(self) -> Option<u32> {
        if self.mask != 0 && self.payload & UNIFORM_BIT == 0 {
            Some(self.payload)
        } else {
            None
        }
    }

    /// Every voxel is one solid material: a uniform child.
    pub const fn uniform(self) -> Option<MaterialId> {
        if self.mask == u64::MAX {
            self.material()
        } else {
            None
        }
    }

    /// Mixed content: neither air nor uniform. These are the tree's leaves.
    pub const fn is_leaf(self) -> bool {
        self.mask != 0 && self.uniform().is_none()
    }
}

/// The header word of a palette block.
pub(crate) const fn header(palette_len: usize, index_bits: u32) -> u32 {
    palette_len as u32 | (index_bits << 8)
}

/// Palette length stored in a header.
pub(crate) const fn palette_len(header: u32) -> usize {
    (header & 0xFF) as usize
}

/// Index bit width stored in a header.
pub(crate) const fn index_bits(header: u32) -> u32 {
    (header >> 8) & 0xFF
}

/// The smallest supported width (0, 1, 2, 4 or 8 bits) that indexes a
/// palette of `palette_len` entries.
pub(crate) const fn width_for(palette_len: usize) -> u32 {
    match palette_len {
        0 | 1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    }
}

/// Words holding a palette of `len` entries.
pub(crate) const fn palette_words(len: usize) -> usize {
    len.div_ceil(2)
}

/// Words holding `count` indices of `bits` bits each.
pub(crate) const fn index_words(count: usize, bits: u32) -> usize {
    (count * bits as usize).div_ceil(32)
}

/// Words in the palette block with this header, for a leaf with this mask.
pub(crate) const fn block_len(mask: u64, header: u32) -> usize {
    1 + palette_words(palette_len(header))
        + index_words(mask.count_ones() as usize, index_bits(header))
}

/// Palette entry `i` of a palette block.
#[inline]
pub(crate) fn palette_entry(block: &[u32], i: usize) -> MaterialId {
    MaterialId((block[1 + i / 2] >> (16 * (i % 2))) as u16)
}

/// The material of the `k`-th solid voxel of a palette block.
#[inline]
pub(crate) fn solid_voxel(block: &[u32], k: usize) -> MaterialId {
    let header = block[0];
    let bits = index_bits(header) as usize;
    let at = k * bits;
    let word = block[1 + palette_words(palette_len(header)) + at / 32];
    let index = (word >> (at % 32)) & ((1 << bits) - 1);
    palette_entry(block, index as usize)
}

/// Every voxel of a leaf. `block` is its palette block, empty when the
/// record has none.
pub(crate) fn decode(record: Record, block: &[u32]) -> Voxels {
    let mut voxels: Voxels = [MaterialId::AIR; TREE_FANOUT as usize];
    match record.material() {
        Some(m) => slots(record.mask).for_each(|slot| voxels[slot as usize] = m),
        None => {
            for (k, slot) in slots(record.mask).enumerate() {
                voxels[slot as usize] = solid_voxel(block, k);
            }
        }
    }
    voxels
}

/// A palette block ready to store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaletteBlock {
    words: [u32; MAX_PALETTE_BLOCK],
    len: usize,
}

impl PaletteBlock {
    pub fn words(&self) -> &[u32] {
        &self.words[..self.len]
    }
}

/// What 64 voxels canonicalize to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackedLeaf {
    /// Air, uniform, or one material: a record alone.
    Inline(Record),
    /// Two or more materials: a record whose payload is the offset of this
    /// block, once stored.
    Palette { mask: u64, block: PaletteBlock },
}

/// Packs 64 voxels into canonical form: no palette for zero or one
/// material, else exactly the materials present, ascending, at the minimal
/// index width.
pub(crate) fn encode(voxels: &Voxels) -> PackedLeaf {
    let mut mask = 0u64;
    let mut palette = [MaterialId::AIR; MAX_PALETTE];
    let mut len = 0;
    for (slot, &m) in (0..).zip(voxels) {
        if !m.is_air() {
            mask |= 1 << slot;
            palette[len] = m;
            len += 1;
        }
    }
    if mask == 0 {
        return PackedLeaf::Inline(Record::AIR);
    }
    let first = palette[0];
    if palette[..len].iter().all(|&m| m == first) {
        return PackedLeaf::Inline(Record::single(mask, first));
    }
    palette[..len].sort_unstable();
    let len = dedup_sorted(&mut palette[..len]);
    let palette = &palette[..len];

    let bits = width_for(len);
    let index_start = 1 + palette_words(len);
    let mut words = [0u32; MAX_PALETTE_BLOCK];
    words[0] = header(len, bits);
    for (i, m) in palette.iter().enumerate() {
        words[1 + i / 2] |= u32::from(m.0) << (16 * (i % 2));
    }
    for (k, slot) in slots(mask).enumerate() {
        let index = palette
            .binary_search(&voxels[slot as usize])
            .expect("every solid voxel is in the palette") as u32;
        let at = k * bits as usize;
        words[index_start + at / 32] |= index << (at % 32);
    }
    let len = index_start + index_words(mask.count_ones() as usize, bits);
    PackedLeaf::Palette {
        mask,
        block: PaletteBlock { words, len },
    }
}

/// Removes repeats from a sorted slice in place; returns the unique length.
fn dedup_sorted(sorted: &mut [MaterialId]) -> usize {
    let mut unique = 0;
    for i in 0..sorted.len() {
        if unique == 0 || sorted[unique - 1] != sorted[i] {
            sorted[unique] = sorted[i];
            unique += 1;
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette_leaf(voxels: &Voxels) -> (u64, PaletteBlock) {
        match encode(voxels) {
            PackedLeaf::Palette { mask, block } => (mask, block),
            other => panic!("expected a palette leaf, got {other:?}"),
        }
    }

    /// Decodes a packed leaf, as if its block were stored at offset 0.
    fn round_trip(voxels: &Voxels) -> Voxels {
        match encode(voxels) {
            PackedLeaf::Inline(r) => decode(r, &[]),
            PackedLeaf::Palette { mask, block } => {
                decode(Record { mask, payload: 0 }, block.words())
            }
        }
    }

    /// A leaf using each of materials `1..=n`, with one air voxel when
    /// `with_air` (which must leave every material present).
    fn leaf_with_palette(n: u16, with_air: bool) -> Voxels {
        let mut voxels: Voxels = std::array::from_fn(|slot| MaterialId(1 + slot as u16 % n));
        if with_air {
            voxels[63] = MaterialId::AIR;
        }
        voxels
    }

    #[test]
    fn width_is_the_smallest_that_indexes_the_palette() {
        let expected = [
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 4),
            (16, 4),
            (17, 8),
            (64, 8),
        ];
        for (len, bits) in expected {
            assert_eq!(width_for(len), bits, "palette of {len}");
        }
    }

    #[test]
    fn round_trips_at_every_bit_width() {
        for (n, bits) in [(2, 1), (3, 2), (4, 2), (5, 4), (16, 4), (17, 8), (64, 8)] {
            for with_air in [false, true] {
                if n == 64 && with_air {
                    continue; // 64 materials leave no voxel to clear.
                }
                let voxels = leaf_with_palette(n, with_air);
                let (mask, block) = palette_leaf(&voxels);
                let h = block.words()[0];
                assert_eq!(index_bits(h), bits, "palette of {n}");
                assert_eq!(palette_len(h), n as usize);
                assert_eq!(block_len(mask, h), block.words().len());
                assert_eq!(round_trip(&voxels), voxels, "palette of {n}");
            }
        }
    }

    #[test]
    fn one_material_is_an_inline_record() {
        let mut voxels = [MaterialId::AIR; 64];
        voxels[5] = MaterialId(9);
        voxels[63] = MaterialId(9);
        let r = Record::single((1 << 5) | (1 << 63), MaterialId(9));
        assert_eq!(encode(&voxels), PackedLeaf::Inline(r));
        assert_eq!(r.words(), [1 << 5, 1 << 31, UNIFORM_BIT | 9]);
        assert_eq!(Record::read(&r.words()), r);
        assert_eq!(round_trip(&voxels), voxels);
        assert!(r.is_leaf());
        assert_eq!((r.uniform(), r.palette_base()), (None, None));
    }

    #[test]
    fn uniform_and_empty_voxels_are_not_leaves() {
        assert_eq!(
            encode(&[MaterialId::AIR; 64]),
            PackedLeaf::Inline(Record::AIR)
        );
        let full = Record::filled(MaterialId(4));
        assert_eq!(encode(&[MaterialId(4); 64]), PackedLeaf::Inline(full));
        assert_eq!(full.uniform(), Some(MaterialId(4)));
        assert!(!full.is_leaf() && !Record::AIR.is_leaf());
        assert_eq!(Record::filled(MaterialId::AIR), Record::AIR);
    }

    #[test]
    fn palette_is_sorted_unique_and_exactly_what_is_present() {
        let mut voxels = [MaterialId::AIR; 64];
        voxels[3] = MaterialId(900);
        voxels[10] = MaterialId(5);
        voxels[11] = MaterialId(900);
        voxels[60] = MaterialId(42);
        let (_, block) = palette_leaf(&voxels);
        let palette: Vec<_> = (0..palette_len(block.words()[0]))
            .map(|i| palette_entry(block.words(), i))
            .collect();
        assert_eq!(palette, [MaterialId(5), MaterialId(42), MaterialId(900)]);
    }

    #[test]
    fn layout_is_header_then_low_half_first_then_lsb_first() {
        // Slots 0, 1, 2 hold materials 7, 3, 7: palette [3, 7], width 1,
        // indices 1, 0, 1 packed LSB-first: 0b101.
        let mut voxels = [MaterialId::AIR; 64];
        voxels[0] = MaterialId(7);
        voxels[1] = MaterialId(3);
        voxels[2] = MaterialId(7);
        let (mask, block) = palette_leaf(&voxels);
        assert_eq!(mask, 0b111);
        assert_eq!(block.words(), [2 | (1 << 8), 3 | (7 << 16), 0b101]);
        let r = Record { mask, payload: 40 };
        assert_eq!(r.palette_base(), Some(40));
        assert_eq!(r.material(), None);
    }

    #[test]
    fn the_largest_leaf_fills_the_maximum_block() {
        let voxels: Voxels = std::array::from_fn(|slot| MaterialId(slot as u16 + 1));
        let (_, block) = palette_leaf(&voxels);
        assert_eq!(block.words().len(), MAX_PALETTE_BLOCK);
        assert_eq!(round_trip(&voxels), voxels);
    }
}
