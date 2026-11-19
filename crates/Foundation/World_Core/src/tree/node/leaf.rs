//! Leaf blocks: a palette of materials and one packed index per solid voxel.
//!
//! A leaf's block at `words[base..]` is the palette, two u16 material ids per
//! word (low half first, an odd palette leaves the last high half zero), then
//! one index per solid voxel in slot order, packed LSB-first into whole words.
//! The index width is the smallest of 0, 1, 2, 4 or 8 bits that can index the
//! palette, so an index never straddles a word. With one material the width is
//! 0 and no index words follow. The node's `info` holds the palette length in
//! its low byte and the width in its second byte (decision 0004).

use super::{Packed, PackedNode, slots};
use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;

/// The 64 voxels of a leaf in slot order, air included.
pub(crate) type Voxels = [MaterialId; TREE_FANOUT as usize];

/// The longest palette: every voxel a different material.
const MAX_PALETTE: usize = TREE_FANOUT as usize;

/// The longest leaf block: a full palette plus 64 eight-bit indices.
const MAX_BLOCK_LEN: usize = palette_words(MAX_PALETTE) + index_words(64, 8);

const _: () = assert!(MAX_BLOCK_LEN == 48);
const _: () = assert!(MAX_BLOCK_LEN <= TREE_FANOUT as usize);

/// The `info` word of a leaf.
pub(crate) const fn info(palette_len: usize, index_bits: u32) -> u32 {
    palette_len as u32 | (index_bits << 8)
}

/// Palette length stored in a leaf's `info`.
pub(crate) const fn palette_len(info: u32) -> usize {
    (info & 0xFF) as usize
}

/// Index bit width stored in a leaf's `info`.
pub(crate) const fn index_bits(info: u32) -> u32 {
    (info >> 8) & 0xFF
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

/// Words in the block of a leaf with this mask and info.
pub(crate) const fn block_len(mask: u64, info: u32) -> usize {
    palette_words(palette_len(info)) + index_words(mask.count_ones() as usize, index_bits(info))
}

/// Palette entry `i` of a leaf block.
pub(crate) fn palette_entry(block: &[u32], i: usize) -> MaterialId {
    MaterialId((block[i / 2] >> (16 * (i % 2))) as u16)
}

/// The palette index of the `k`-th solid voxel of a leaf block.
pub(crate) fn packed_index(block: &[u32], info: u32, k: usize) -> usize {
    let bits = index_bits(info);
    if bits == 0 {
        return 0;
    }
    let at = k * bits as usize;
    let word = block[palette_words(palette_len(info)) + at / 32];
    ((word >> (at % 32)) & ((1 << bits) - 1)) as usize
}

/// Every voxel of a stored leaf.
pub(crate) fn decode(mask: u64, info: u32, block: &[u32]) -> Voxels {
    let mut voxels: Voxels = [MaterialId::AIR; TREE_FANOUT as usize];
    for (k, slot) in slots(mask).enumerate() {
        voxels[slot as usize] = palette_entry(block, packed_index(block, info, k));
    }
    voxels
}

/// Packs 64 voxels into canonical form: all one material collapses to
/// [`Packed::Filled`], anything else becomes a leaf with exactly the materials
/// present, ascending, at the minimal index width.
pub(crate) fn encode(voxels: &Voxels) -> Packed {
    let mask = voxels
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.is_air())
        .fold(0u64, |mask, (slot, _)| mask | (1 << slot));
    if mask == 0 {
        return Packed::Filled(MaterialId::AIR);
    }

    let mut palette = [MaterialId::AIR; MAX_PALETTE];
    let mut len = 0;
    for slot in slots(mask) {
        palette[len] = voxels[slot as usize];
        len += 1;
    }
    palette[..len].sort_unstable();
    let len = dedup_sorted(&mut palette[..len]);
    let palette = &palette[..len];
    if mask == u64::MAX && len == 1 {
        return Packed::Filled(palette[0]);
    }

    let bits = width_for(len);
    let index_start = palette_words(len);
    let mut block = [0u32; MAX_BLOCK_LEN];
    for (i, m) in palette.iter().enumerate() {
        block[i / 2] |= u32::from(m.0) << (16 * (i % 2));
    }
    if bits > 0 {
        for (k, slot) in slots(mask).enumerate() {
            let index = palette
                .binary_search(&voxels[slot as usize])
                .expect("every solid voxel is in the palette") as u32;
            let at = k * bits as usize;
            block[index_start + at / 32] |= index << (at % 32);
        }
    }
    let used = index_start + index_words(mask.count_ones() as usize, bits);
    Packed::Node(PackedNode::new(mask, info(len, bits), &block[..used]))
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

    fn leaf(voxels: &Voxels) -> PackedNode {
        match encode(voxels) {
            Packed::Node(p) => p,
            other => panic!("expected a leaf, got {other:?}"),
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
        for (n, bits) in [
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 4),
            (16, 4),
            (17, 8),
            (64, 8),
        ] {
            for with_air in [false, true] {
                if (n == 1 && !with_air) || (n == 64 && with_air) {
                    // One material fills a uniform child, not a leaf; 64
                    // materials leave no voxel to clear.
                    continue;
                }
                let voxels = leaf_with_palette(n, with_air);
                let p = leaf(&voxels);
                assert_eq!(index_bits(p.info), bits, "palette of {n}");
                assert_eq!(palette_len(p.info), n as usize);
                assert_eq!(block_len(p.mask, p.info), p.words().len());
                assert_eq!(decode(p.mask, p.info, p.words()), voxels, "palette of {n}");
            }
        }
    }

    #[test]
    fn palette_is_sorted_unique_and_exactly_what_is_present() {
        let mut voxels = [MaterialId::AIR; 64];
        voxels[3] = MaterialId(900);
        voxels[10] = MaterialId(5);
        voxels[11] = MaterialId(900);
        voxels[60] = MaterialId(42);
        let p = leaf(&voxels);
        let palette: Vec<_> = (0..palette_len(p.info))
            .map(|i| palette_entry(p.words(), i))
            .collect();
        assert_eq!(palette, [MaterialId(5), MaterialId(42), MaterialId(900)]);
    }

    #[test]
    fn layout_is_low_half_first_and_lsb_first() {
        // Slots 0, 1, 2 hold materials 7, 3, 7: palette [3, 7], width 1,
        // indices 1, 0, 1 packed LSB-first: 0b101.
        let mut voxels = [MaterialId::AIR; 64];
        voxels[0] = MaterialId(7);
        voxels[1] = MaterialId(3);
        voxels[2] = MaterialId(7);
        let p = leaf(&voxels);
        assert_eq!(p.mask, 0b111);
        assert_eq!(p.info, 2 | (1 << 8));
        assert_eq!(p.words(), [3 | (7 << 16), 0b101]);
    }

    #[test]
    fn a_single_material_needs_no_index_words() {
        let mut voxels = [MaterialId::AIR; 64];
        voxels[5] = MaterialId(9);
        voxels[63] = MaterialId(9);
        let p = leaf(&voxels);
        assert_eq!(p.info, 1);
        assert_eq!(p.words(), [9]);
    }

    #[test]
    fn uniform_and_empty_voxels_are_not_leaves() {
        assert_eq!(
            encode(&[MaterialId::AIR; 64]),
            Packed::Filled(MaterialId::AIR)
        );
        assert_eq!(encode(&[MaterialId(4); 64]), Packed::Filled(MaterialId(4)));
    }

    #[test]
    fn the_largest_leaf_fills_the_maximum_block() {
        let voxels: Voxels = std::array::from_fn(|slot| MaterialId(slot as u16 + 1));
        let p = leaf(&voxels);
        assert_eq!(p.words().len(), MAX_BLOCK_LEN);
        assert_eq!(decode(p.mask, p.info, p.words()), voxels);
    }
}
