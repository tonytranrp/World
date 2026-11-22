//! Leaves: the 4x4x4-voxel children of a level-1 node, stored as inline
//! records with an optional palette block (layout in the module above).

use super::{PAIR_WORDS, RECORD_WORDS, UNIFORM_BIT, slots};
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

/// One leaf as its level-1 parent stores it: the record's mask and payload,
/// and for a two-material leaf its indices. A zero mask is an air leaf,
/// which has no record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Record {
    /// Bit `j` is set when voxel `j` is solid.
    pub mask: u64,
    /// [`UNIFORM_BIT`] and a material, or the offset of a palette block; for
    /// a two-material leaf, its palette word (decision 0031).
    pub payload: u32,
    /// For a two-material leaf, stored inline (decision 0031): its one-bit
    /// indices, one per solid voxel, never zero since both materials are
    /// present. Zero for every other leaf.
    pub indices: u64,
}

impl Record {
    pub const AIR: Self = Self {
        mask: 0,
        payload: 0,
        indices: 0,
    };

    /// The two-material leaf of solid voxels `mask`, palette word `palette`
    /// (the lower id in the low half) and one-bit `indices`.
    pub const fn pair(mask: u64, palette: u32, indices: u64) -> Self {
        Self {
            mask,
            payload: palette,
            indices,
        }
    }

    /// Whether this is a two-material leaf stored inline.
    pub const fn is_pair(self) -> bool {
        self.indices != 0
    }

    /// Whether this is a leaf of two or more materials, which a brick
    /// stores in [`PAIR_WORDS`] words, in its mixed mask (decision 0035).
    pub const fn is_mixed(self) -> bool {
        self.mask != 0 && (self.indices != 0 || self.payload & UNIFORM_BIT == 0)
    }

    /// Reads a stored leaf of the mixed mask from its [`PAIR_WORDS`] words:
    /// a two-material leaf, or (zero indices) the record of a wider palette.
    pub fn read_pair(words: &[u32]) -> Self {
        Self::pair(
            u64::from(words[0]) | u64::from(words[1]) << 32,
            words[2],
            u64::from(words[3]) | u64::from(words[4]) << 32,
        )
    }

    /// The [`PAIR_WORDS`] stored words of a mixed leaf: its mask, then a
    /// two-material leaf's palette block as the GPU lays it out (the palette
    /// word, then the indices), or a wider palette's block offset and two
    /// zero words.
    pub const fn pair_words(self) -> [u32; PAIR_WORDS] {
        [
            self.mask as u32,
            (self.mask >> 32) as u32,
            self.payload,
            self.indices as u32,
            (self.indices >> 32) as u32,
        ]
    }

    /// A two-material leaf's palette block, as a block stored apart would
    /// hold it: the palette word and the index words its voxels use.
    pub fn pair_block(self) -> ([u32; 3], usize) {
        let words = [
            self.payload,
            self.indices as u32,
            (self.indices >> 32) as u32,
        ];
        (words, 1 + index_words(self.mask.count_ones() as usize, 1))
    }

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
            indices: 0,
        }
    }

    /// The record of solid voxels `mask` whose palette block is at `base`.
    pub const fn palette(mask: u64, base: u32) -> Self {
        Self {
            mask,
            payload: base,
            indices: 0,
        }
    }

    /// Reads a stored record (not a two-material leaf) from its three words.
    pub fn read(words: &[u32]) -> Self {
        Self::palette(u64::from(words[0]) | u64::from(words[1]) << 32, words[2])
    }

    /// The three stored words of a record that is not a two-material leaf.
    pub const fn words(self) -> [u32; RECORD_WORDS] {
        [self.mask as u32, (self.mask >> 32) as u32, self.payload]
    }

    /// The one material of every solid voxel, when there is no palette.
    pub const fn material(self) -> Option<MaterialId> {
        if self.indices == 0 && self.payload & UNIFORM_BIT != 0 {
            Some(MaterialId(self.payload as u16))
        } else {
            None
        }
    }

    /// The offset of the palette block, when the leaf has one stored apart.
    pub const fn palette_base(self) -> Option<u32> {
        if self.mask != 0 && self.indices == 0 && self.payload & UNIFORM_BIT == 0 {
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

/// The header word of a palette block of three or more materials.
pub(crate) const fn header(palette_len: usize, index_bits: u32) -> u32 {
    palette_len as u32 | (index_bits << 8)
}

/// Whether `first`, a palette block's first word, is the palette of a
/// two-material block, which has no header (decision 0025): its high half
/// is the larger id, at least 2, where a header's high half is zero.
#[inline(always)]
pub(crate) const fn is_pair(first: u32) -> bool {
    first >> 16 != 0
}

/// Palette length of the block whose first word is `first`.
pub(crate) const fn palette_len(first: u32) -> usize {
    if is_pair(first) {
        2
    } else {
        (first & 0xFF) as usize
    }
}

/// Index bit width of the block whose first word is `first`.
pub(crate) const fn index_bits(first: u32) -> u32 {
    if is_pair(first) {
        1
    } else {
        (first >> 8) & 0xFF
    }
}

/// Where the palette of the block whose first word is `first` starts.
const fn palette_start(first: u32) -> usize {
    if is_pair(first) { 0 } else { 1 }
}

/// Where the indices of the block whose first word is `first` start.
pub(crate) const fn index_start(first: u32) -> usize {
    palette_start(first) + palette_words(palette_len(first))
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

/// Words in the palette block whose first word is `first`, for a leaf with
/// this mask.
pub(crate) const fn block_len(mask: u64, first: u32) -> usize {
    index_start(first) + index_words(mask.count_ones() as usize, index_bits(first))
}

/// Words in a palette block of `palette_len` (2 to 64) materials for `solid`
/// voxels.
#[cfg(test)]
pub(crate) const fn block_len_for(palette_len: usize, solid: usize) -> usize {
    let header = if palette_len == 2 { 0 } else { 1 };
    header + palette_words(palette_len) + index_words(solid, width_for(palette_len))
}

/// Palette entry `i` of a palette block.
#[inline]
pub(crate) fn palette_entry(block: &[u32], i: usize) -> MaterialId {
    MaterialId((block[palette_start(block[0]) + i / 2] >> (16 * (i % 2))) as u16)
}

/// The material of the `k`-th solid voxel of a palette block.
#[inline]
pub(crate) fn solid_voxel(block: &[u32], k: usize) -> MaterialId {
    if is_pair(block[0]) {
        pair_voxel(block, k)
    } else {
        headed_voxel(block, k)
    }
}

/// [`solid_voxel`] of a two-material block: one bit each, the palette in
/// the first word.
#[inline]
pub(crate) fn pair_voxel(block: &[u32], k: usize) -> MaterialId {
    let index = (block[1 + k / 32] >> (k % 32)) & 1;
    MaterialId((block[0] >> (16 * index)) as u16)
}

/// [`solid_voxel`] of a block with a header (three or more materials).
#[inline]
pub(crate) fn headed_voxel(block: &[u32], k: usize) -> MaterialId {
    let header = block[0];
    let bits = ((header >> 8) & 0xFF) as usize;
    let at = k * bits;
    let word = block[1 + palette_words((header & 0xFF) as usize) + at / 32];
    let index = (word >> (at % 32)) & ((1 << bits) - 1);
    MaterialId((block[1 + index as usize / 2] >> (16 * (index % 2))) as u16)
}

/// Passes the solid voxels of a packed leaf to `each`: every palette entry
/// that holds any, with how many. `block` is the leaf's whole palette block
/// and `solid` its number of solid voxels. Entries a stored palette holds
/// without using them are skipped. Two-material leaves, the common kind,
/// cost a popcount per index word.
pub(crate) fn count_solid(block: &[u32], solid: u32, each: &mut impl FnMut(MaterialId, u32)) {
    let header = block[0];
    let len = palette_len(header);
    let bits = index_bits(header);
    let indices = &block[index_start(header)..];
    let mut emit = |i: usize, n: u32| {
        if n > 0 && i < len {
            each(palette_entry(block, i), n);
        }
    };
    match bits {
        0 => emit(0, solid),
        1 => {
            // Index 1 is a set bit; only the first `solid` bits are indices.
            let (whole, rest) = ((solid / 32) as usize, solid % 32);
            let mut ones: u32 = indices[..whole].iter().map(|w| w.count_ones()).sum();
            if rest > 0 {
                ones += (indices[whole] & ((1 << rest) - 1)).count_ones();
            }
            emit(0, solid - ones);
            emit(1, ones);
        }
        _ => {
            let mut counts = [0u32; MAX_PALETTE];
            let index_mask = (1u32 << bits) - 1;
            for k in 0..solid {
                let at = k * bits;
                let index = (indices[(at / 32) as usize] >> (at % 32)) & index_mask;
                // Indices stay below the palette's length, at most 64.
                if let Some(c) = counts.get_mut(index as usize) {
                    *c += 1;
                }
            }
            for (i, &n) in counts.iter().enumerate().take(len) {
                emit(i, n);
            }
        }
    }
}

/// Every voxel of a leaf. `block` is its palette block, empty when the
/// record has none.
pub(crate) fn decode(record: Record, block: &[u32]) -> Voxels {
    if record.is_pair() {
        let (words, _) = record.pair_block();
        return decode(Record::palette(record.mask, 0), &words);
    }
    let mut voxels: Voxels = [MaterialId::AIR; TREE_FANOUT as usize];
    match record.material() {
        Some(m) => slots(record.mask).for_each(|slot| voxels[slot as usize] = m),
        // An air record has no material and no block.
        None if record.mask == 0 => {}
        None => {
            // The layout once, then one index per solid voxel.
            let first = block[0];
            let bits = index_bits(first) as usize;
            let (palette, indices) = (palette_start(first), index_start(first));
            for (k, slot) in slots(record.mask).enumerate() {
                let at = k * bits;
                let index = (block[indices + at / 32] >> (at % 32)) as usize & ((1 << bits) - 1);
                voxels[slot as usize] =
                    MaterialId((block[palette + index / 2] >> (16 * (index % 2))) as u16);
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

    /// The one-bit indices of a two-material block (words past its length
    /// are zero).
    pub fn pair_indices(&self) -> u64 {
        u64::from(self.words[1]) | u64::from(self.words[2]) << 32
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
    // Branch-free passes first, 16 voxels at a time so they vectorize: the
    // solid mask and the smallest and largest solid material. Most leaves
    // stop at one of the two common shapes below.
    let mask = bits(voxels, |m| m != 0);
    if mask == 0 {
        return PackedLeaf::Inline(Record::AIR);
    }
    let (mut lo, mut hi) = (u16::MAX, 0u16);
    for &m in voxels {
        lo = lo.min(if m.is_air() { u16::MAX } else { m.0 });
        hi = hi.max(m.0);
    }
    if lo == hi {
        return PackedLeaf::Inline(Record::single(mask, MaterialId(lo)));
    }
    // Exactly two materials: one index bit per solid voxel, set for `hi`.
    let others = bits(voxels, |m| m != 0 && m != lo && m != hi);
    if others == 0 {
        let high = bits(voxels, |m| m == hi);
        return two_materials(mask, MaterialId(lo), MaterialId(hi), high);
    }
    if hi - lo < 64 {
        return close_materials(voxels, mask, lo);
    }
    many_materials(voxels, mask)
}

/// The palette leaf of solid voxels `mask` holding three or more materials,
/// all within 64 ids from `lo`, the smallest: a bit per id says which are
/// present, which lists the palette in ascending order and gives each
/// material's index as the number of present ids below it.
fn close_materials(voxels: &Voxels, mask: u64, lo: u16) -> PackedLeaf {
    let mut present = 0u64;
    for &m in voxels {
        // Air voxels shift a zero; the mask keeps their shift in range.
        present |= u64::from(!m.is_air()) << (m.0.wrapping_sub(lo) & 63);
    }
    let len = present.count_ones() as usize;
    let bits = width_for(len);
    let index_start = 1 + palette_words(len);
    let mut words = [0u32; MAX_PALETTE_BLOCK];
    words[0] = header(len, bits);
    let mut rest = present;
    for i in 0..len {
        let m = lo + rest.trailing_zeros() as u16;
        rest &= rest - 1;
        words[1 + i / 2] |= u32::from(m) << (16 * (i % 2));
    }
    for (k, slot) in slots(mask).enumerate() {
        let below = (1u64 << (voxels[slot as usize].0 - lo)) - 1;
        let index = (present & below).count_ones();
        let at = k * bits as usize;
        words[index_start + at / 32] |= index << (at % 32);
    }
    let len = index_start + index_words(mask.count_ones() as usize, bits);
    PackedLeaf::Palette {
        mask,
        block: PaletteBlock { words, len },
    }
}

/// Bit `slot` set where `test` holds for voxel `slot`'s material id.
#[inline(always)]
fn bits(voxels: &Voxels, test: impl Fn(u16) -> bool) -> u64 {
    let mut out = 0u64;
    for (i, chunk) in voxels.chunks_exact(16).enumerate() {
        let mut word = 0u16;
        for (j, m) in chunk.iter().enumerate() {
            word |= u16::from(test(m.0)) << j;
        }
        out |= u64::from(word) << (16 * i);
    }
    out
}

/// The palette leaf of solid voxels `mask` made of `lo` and `hi` (`lo < hi`),
/// with `hi` where `high` is set.
fn two_materials(mask: u64, lo: MaterialId, hi: MaterialId, high: u64) -> PackedLeaf {
    // Index `k` is the bit of the `k`-th solid voxel.
    let mut indices = 0u64;
    for (k, slot) in slots(mask).enumerate() {
        indices |= (high >> slot & 1) << k;
    }
    // No header: the palette word says it is a two-material block.
    let mut words = [0u32; MAX_PALETTE_BLOCK];
    words[0] = u32::from(lo.0) | u32::from(hi.0) << 16;
    words[1] = indices as u32;
    words[2] = (indices >> 32) as u32;
    let len = 1 + index_words(mask.count_ones() as usize, 1);
    PackedLeaf::Palette {
        mask,
        block: PaletteBlock { words, len },
    }
}

/// The palette leaf of solid voxels `mask` holding three or more materials,
/// spread over more than 64 ids.
fn many_materials(voxels: &Voxels, mask: u64) -> PackedLeaf {
    // The distinct solid materials in order of first appearance; runs of
    // one material skip the scan.
    let mut distinct = [MaterialId::AIR; MAX_PALETTE];
    let mut len = 0;
    let mut last = MaterialId::AIR;
    for &m in voxels {
        if m != last && !m.is_air() {
            last = m;
            if !distinct[..len].contains(&m) {
                distinct[len] = m;
                len += 1;
            }
        }
    }
    let palette = &mut distinct[..len];
    palette.sort_unstable();
    let palette = &*palette;

    let bits = width_for(len);
    let index_start = 1 + palette_words(len);
    let mut words = [0u32; MAX_PALETTE_BLOCK];
    words[0] = header(len, bits);
    for (i, m) in palette.iter().enumerate() {
        words[1 + i / 2] |= u32::from(m.0) << (16 * (i % 2));
    }
    // Runs of one material share their palette lookup.
    let (mut last, mut index) = (MaterialId::AIR, 0u32);
    for (k, slot) in slots(mask).enumerate() {
        let m = voxels[slot as usize];
        if m != last {
            last = m;
            index = palette
                .binary_search(&m)
                .expect("every solid voxel is in the palette") as u32;
        }
        let at = k * bits as usize;
        words[index_start + at / 32] |= index << (at % 32);
    }
    let len = index_start + index_words(mask.count_ones() as usize, bits);
    PackedLeaf::Palette {
        mask,
        block: PaletteBlock { words, len },
    }
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
            PackedLeaf::Palette { mask, block } => decode(Record::palette(mask, 0), block.words()),
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
                let solid = mask.count_ones() as usize;
                assert_eq!(block_len_for(n as usize, solid), block.words().len());
                assert_eq!(is_pair(h), n == 2, "only two materials go headerless");
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
        // indices 1, 0, 1 packed LSB-first: 0b101. Two materials have no
        // header.
        let mut voxels = [MaterialId::AIR; 64];
        voxels[0] = MaterialId(7);
        voxels[1] = MaterialId(3);
        voxels[2] = MaterialId(7);
        let (mask, block) = palette_leaf(&voxels);
        assert_eq!(mask, 0b111);
        assert_eq!(block.words(), [3 | (7 << 16), 0b101]);
        assert_eq!(solid_voxel(block.words(), 1), MaterialId(3));
        assert_eq!(solid_voxel(block.words(), 2), MaterialId(7));
        // Three: header `3 | 2 << 8`, palette [3, 7], [9], indices 2, 0, 1, 2.
        voxels[3] = MaterialId(9);
        voxels[0] = MaterialId(9);
        let (_, block) = palette_leaf(&voxels);
        assert_eq!(
            block.words(),
            [3 | (2 << 8), 3 | (7 << 16), 9, 2 | 1 << 4 | 2 << 6]
        );
        assert_eq!(index_start(block.words()[0]), 3);
        let r = Record::palette(mask, 40);
        assert_eq!(r.palette_base(), Some(40));
        assert_eq!(r.material(), None);
    }

    /// The straightforward encoder: sort every solid voxel's material,
    /// dedup, then binary-search each voxel.
    fn reference_encode(voxels: &Voxels) -> PackedLeaf {
        let mask = (0..64).fold(0u64, |m, s| m | u64::from(!voxels[s].is_air()) << s);
        let mut palette: Vec<MaterialId> = voxels.iter().copied().filter(|m| !m.is_air()).collect();
        palette.sort_unstable();
        palette.dedup();
        match palette.len() {
            0 => return PackedLeaf::Inline(Record::AIR),
            1 => return PackedLeaf::Inline(Record::single(mask, palette[0])),
            _ => {}
        }
        let len = palette.len();
        let bits = width_for(len);
        let palette_start = usize::from(len > 2);
        let index_start = palette_start + palette_words(len);
        let mut words = [0u32; MAX_PALETTE_BLOCK];
        if len > 2 {
            words[0] = header(len, bits);
        }
        for (i, m) in palette.iter().enumerate() {
            words[palette_start + i / 2] |= u32::from(m.0) << (16 * (i % 2));
        }
        for (k, slot) in slots(mask).enumerate() {
            let index = palette.binary_search(&voxels[slot as usize]).unwrap() as u32;
            let at = k * bits as usize;
            words[index_start + at / 32] |= index << (at % 32);
        }
        let len = index_start + index_words(mask.count_ones() as usize, bits);
        PackedLeaf::Palette {
            mask,
            block: PaletteBlock { words, len },
        }
    }

    proptest::proptest! {
        #[test]
        fn encode_matches_the_reference(
            // Few materials with long runs, like terrain, up to every slot
            // its own material.
            spread in proptest::prop_oneof![1u16..5, 1u16..80, 60u16..400],
            // Ids from air up to KEEP (u16::MAX), which saturation makes
            // common at the top.
            base in proptest::prop_oneof![
                proptest::strategy::Just(0u16),
                1000u16..2000,
                (u16::MAX - 400)..=u16::MAX,
            ],
            run in 1usize..20,
            seed: u64,
        ) {
            let mut state = seed | 1;
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            };
            let mut voxels: Voxels = [MaterialId::AIR; 64];
            let mut m = MaterialId::AIR;
            for (slot, v) in voxels.iter_mut().enumerate() {
                if slot % run == 0 {
                    m = MaterialId(base.saturating_add((next() % u64::from(spread)) as u16));
                }
                *v = m;
            }
            proptest::prop_assert_eq!(encode(&voxels), reference_encode(&voxels));
        }
    }

    #[test]
    fn the_largest_leaf_fills_the_maximum_block() {
        let voxels: Voxels = std::array::from_fn(|slot| MaterialId(slot as u16 + 1));
        let (_, block) = palette_leaf(&voxels);
        assert_eq!(block.words().len(), MAX_PALETTE_BLOCK);
        assert_eq!(round_trip(&voxels), voxels);
    }
}
