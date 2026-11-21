//! The brick block encoder: one level-1 node into its GPU block.

use world_core::MaterialId;
use world_core::tree::NodeView;

use super::summary::{Content, Summary, Tally, content, count_palette_block};
use super::{
    BLOCK_ALIGN_WORDS, BLOCK_DEFAULT_SHIFT, BLOCK_DEFAULT_WORD, BLOCK_FLAG_EDITED,
    BLOCK_FLAGS_SHIFT, BLOCK_HEADER_WORDS, BLOCK_PALETTE_WORDS_SHIFT, LEAF_MASK_WORDS,
    PALETTE_BITS_SHIFT, PALETTE_LEN_MASK, PALETTE_PAIR_SHIFT, PAYLOAD_UNIFORM_BIT,
};

/// The largest block a brick can need, in words: header, 64 partial leaves
/// with a mask and payload each, and 64 palette blocks of up to 64 materials
/// with 8-bit indices (1 + 32 + 16 words).
pub const MAX_BLOCK_WORDS: u32 = BLOCK_HEADER_WORDS + 64 * 3 + 64 * 49;

/// What encoding a brick reported besides its words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodedBrick {
    /// Occupancy of the brick's 64 leaves: the level-1 node's mask.
    pub mask: u64,
    /// Representative material and solid voxel count of the brick.
    pub summary: Summary,
    /// Words written, a multiple of [`BLOCK_ALIGN_WORDS`].
    pub words: u32,
}

/// Bits per palette index for `len` materials (2 to 64): the smallest of
/// 1, 2, 4 and 8 that can index them.
pub const fn index_bits(len: u32) -> u32 {
    if len <= 2 {
        1
    } else if len <= 4 {
        2
    } else if len <= 16 {
        4
    } else {
        8
    }
}

/// Words of a palette block for `len` materials (2 to 64) and `solid`
/// voxels: two materials have no header.
pub const fn palette_block_words(len: u32, solid: u32) -> u32 {
    let header = if len > 2 { 1 } else { 0 };
    header + len.div_ceil(2) + (solid * index_bits(len)).div_ceil(32)
}

/// One leaf's solid voxels and their distinct materials, read once.
struct LeafScan {
    /// Solid voxels in slot order.
    voxels: [MaterialId; 64],
    solid: usize,
    /// Distinct materials, ascending, with their voxel counts.
    palette: [MaterialId; 64],
    counts: [u64; 64],
    len: usize,
}

impl LeafScan {
    fn of<N: NodeView>(leaf: &N, mask: u64) -> Self {
        let mut scan = Self {
            voxels: [MaterialId::AIR; 64],
            solid: 0,
            palette: [MaterialId::AIR; 64],
            counts: [0; 64],
            len: 0,
        };
        let mut bits = mask;
        while bits != 0 {
            let slot = bits.trailing_zeros();
            bits &= bits - 1;
            let m = leaf.voxel(slot);
            scan.voxels[scan.solid] = m;
            scan.solid += 1;
            match scan.palette[..scan.len].binary_search(&m) {
                Ok(i) => scan.counts[i] += 1,
                Err(i) => {
                    scan.palette.copy_within(i..scan.len, i + 1);
                    scan.counts.copy_within(i..scan.len, i + 1);
                    scan.palette[i] = m;
                    scan.counts[i] = 1;
                    scan.len += 1;
                }
            }
        }
        scan
    }
}

/// Appends the block of a level-1 node to `out` and reports it.
///
/// `node` must be a branch whose children are leaves (a level-1 node of a
/// `VoxelSource`); air and uniform level-1 nodes have no block, their record
/// says everything. `scratch` is reused between calls. Every voxel is read
/// at most once; the representative material is counted from the same
/// reads. Full one-material leaves are not read at all: they are in the
/// full mask, with a payload unless the default mask has them too; partial
/// leaves of the default material are a leaf mask without a payload.
pub fn encode_brick<N: NodeView>(
    node: N,
    brick_id: u32,
    edited: bool,
    out: &mut Vec<u32>,
    scratch: &mut Tally,
) -> EncodedBrick {
    debug_assert_eq!(content(&node), Content::Branch);
    let start = out.len();
    let mask = node.mask();
    scratch.clear();

    // The leaves, once, with the material of each one-material partial
    // leaf (the stored palette says so without reading voxels), and the
    // full ones' default: the most common material among them, ties to the
    // lower id.
    let mut leaves = [None; 64];
    let mut single = [MaterialId::AIR; 64];
    let mut full = [(MaterialId::AIR, 0u64); 64];
    let mut materials = 0;
    for (slot, leaf) in node.children() {
        let c = content(&leaf);
        match c {
            Content::Uniform(m) => {
                let bit = 1u64 << slot;
                match full[..materials].iter_mut().find(|(f, _)| *f == m) {
                    Some(entry) => entry.1 |= bit,
                    None => {
                        full[materials] = (m, bit);
                        materials += 1;
                    }
                }
            }
            Content::Leaf => {
                if let [m] = leaf.palette().as_slice() {
                    single[slot as usize] = *m;
                }
            }
            _ => {}
        }
        leaves[slot as usize] = Some((leaf, c));
    }
    let (default, default_full) = full[..materials]
        .iter()
        .copied()
        .max_by(|a, b| a.1.count_ones().cmp(&b.1.count_ones()).then(b.0.cmp(&a.0)))
        .unwrap_or((MaterialId::AIR, 0));
    let full_mask = full[..materials].iter().fold(0, |m, f| m | f.1);
    let mut default_mask = default_full;
    if !default.is_air() {
        for (slot, &m) in single.iter().enumerate() {
            if m == default {
                default_mask |= 1 << slot;
            }
        }
    }
    let partial = mask & !full_mask;
    let paid = mask & !default_mask;
    let p = partial.count_ones();
    let s = paid.count_ones();

    // Header, then room for masks and payloads, filled below.
    out.extend_from_slice(&[
        brick_id,
        0,
        0,
        default_mask as u32,
        (default_mask >> 32) as u32,
        full_mask as u32,
        (full_mask >> 32) as u32,
    ]);
    out.resize(
        start + (BLOCK_HEADER_WORDS + p * LEAF_MASK_WORDS + s) as usize,
        0,
    );
    let masks_at = start + BLOCK_HEADER_WORDS as usize;
    let payloads_at = masks_at + (p * LEAF_MASK_WORDS) as usize;

    if default_full != 0 {
        scratch.add(default, 64 * u64::from(default_full.count_ones()));
    }
    let (mut kp, mut ks) = (0, 0);
    let mut rest = mask & !default_full;
    while rest != 0 {
        let slot = rest.trailing_zeros();
        rest &= rest - 1;
        let Some((leaf, c)) = leaves[slot as usize] else {
            continue;
        };
        let bit = 1u64 << slot;
        let payload = match c {
            Content::Uniform(m) => {
                scratch.add(m, 64);
                PAYLOAD_UNIFORM_BIT | u32::from(m.0)
            }
            _ => {
                let leaf_mask = leaf.mask();
                out[masks_at + 2 * kp] = leaf_mask as u32;
                out[masks_at + 2 * kp + 1] = (leaf_mask >> 32) as u32;
                kp += 1;
                if default_mask & bit != 0 {
                    scratch.add(default, u64::from(leaf_mask.count_ones()));
                    continue;
                }
                leaf_payload(&leaf, leaf_mask, single[slot as usize], start, out, scratch)
            }
        };
        out[payloads_at + ks] = payload;
        ks += 1;
    }
    debug_assert_eq!((kp, ks), (p as usize, s as usize));

    let summary = scratch.summary();
    out[start + BLOCK_DEFAULT_WORD as usize] =
        u32::from(summary.material.0) | u32::from(default.0) << BLOCK_DEFAULT_SHIFT;
    let palette_words = (out.len() - payloads_at - s as usize) as u32;
    let flags = if edited { BLOCK_FLAG_EDITED } else { 0 };
    out[start + 1] = p | palette_words << BLOCK_PALETTE_WORDS_SHIFT | flags << BLOCK_FLAGS_SHIFT;

    let padded = (out.len() - start).next_multiple_of(BLOCK_ALIGN_WORDS as usize);
    out.resize(start + padded, 0);
    EncodedBrick {
        mask,
        summary,
        words: padded as u32,
    }
}

/// The payload of a partial leaf that is not of the default material,
/// appending its palette block to `out` when it has one. `single` is its
/// one material, or air when it has several.
fn leaf_payload<N: NodeView>(
    leaf: &N,
    leaf_mask: u64,
    single: MaterialId,
    start: usize,
    out: &mut Vec<u32>,
    scratch: &mut Tally,
) -> u32 {
    // Most surface leaves hold one material.
    if !single.is_air() {
        scratch.add(single, u64::from(leaf_mask.count_ones()));
        return PAYLOAD_UNIFORM_BIT | u32::from(single.0);
    }
    // A leaf stored packed already has the GPU's palette block layout
    // (decisions 0005 and 0025): copy it and count from its indices.
    if let Some(block) = leaf.palette_block() {
        count_palette_block(block, leaf_mask, scratch);
        let offset = (out.len() - start) as u32;
        out.extend_from_slice(block);
        return offset;
    }
    let scan = LeafScan::of(leaf, leaf_mask);
    for i in 0..scan.len {
        scratch.add(scan.palette[i], scan.counts[i]);
    }
    if scan.len == 1 {
        PAYLOAD_UNIFORM_BIT | u32::from(scan.palette[0].0)
    } else {
        let offset = (out.len() - start) as u32;
        append_palette_block(&scan, out);
        offset
    }
}

// The world tree stores palette blocks with the same header, so they copy.
const _: () = assert!(PALETTE_BITS_SHIFT == 8 && PALETTE_LEN_MASK == 0xFF);
const _: () = assert!(PALETTE_PAIR_SHIFT == 16);

/// Appends one leaf's palette block: a header from three materials on, the
/// palette, the packed indices.
fn append_palette_block(scan: &LeafScan, out: &mut Vec<u32>) {
    let palette = &scan.palette[..scan.len];
    let len = scan.len as u32;
    let bits = index_bits(len);
    if len > 2 {
        out.push(len | bits << PALETTE_BITS_SHIFT);
    }
    for pair in palette.chunks(2) {
        let lo = u32::from(pair[0].0);
        let hi = pair.get(1).map_or(0, |m| u32::from(m.0));
        out.push(lo | hi << 16);
    }
    let first = out.len();
    out.resize(first + (scan.solid as u32 * bits).div_ceil(32) as usize, 0);
    for (k, m) in scan.voxels[..scan.solid].iter().enumerate() {
        // The palette holds every solid material of the leaf, so the search
        // succeeds; 0 is a safe fallback that keeps encoding total.
        let index = palette.binary_search(m).unwrap_or(0) as u32;
        let bit = k as u32 * bits;
        out[first + (bit / 32) as usize] |= index << (bit % 32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brick_cache::format::{
        BLOCK_COUNT_MASK, BLOCK_DEFAULT_MASK_WORD, BLOCK_FULL_MASK_WORD,
    };
    use world_core::{DenseGrid, VoxelPos, VoxelSource};

    fn brick() -> DenseGrid {
        DenseGrid::new(2, VoxelPos::new(0, 0, 0))
    }

    /// Fills leaf `leaf` (0 to 3 along x, at y = z = 0) with `m`.
    fn fill_leaf(g: &mut DenseGrid, leaf: i32, m: u16) {
        for z in 0..4 {
            for y in 0..4 {
                for x in 0..4 {
                    g.set(VoxelPos::new(4 * leaf + x, y, z), MaterialId(m));
                }
            }
        }
    }

    #[test]
    fn index_widths_follow_the_palette_length() {
        assert_eq!(
            [2, 3, 4, 5, 16, 17, 64].map(index_bits),
            [1, 2, 2, 4, 4, 8, 8]
        );
        assert_eq!(palette_block_words(2, 64), 3, "no header for two");
        assert_eq!(palette_block_words(3, 64), 1 + 2 + 4);
    }

    #[test]
    fn single_material_leaves_have_inline_payloads() {
        let mut g = brick();
        g.set(VoxelPos::new(0, 0, 0), MaterialId(4));
        g.set(VoxelPos::new(5, 0, 0), MaterialId(4));
        let mut out = Vec::new();
        let e = encode_brick(g.root(), 7, true, &mut out, &mut Tally::default());
        assert_eq!(e.mask.count_ones(), 2);
        assert_eq!(out[0], 7);
        assert_eq!(out[1] & 0xFF, 2);
        assert_eq!(out[1] >> BLOCK_FLAGS_SHIFT, BLOCK_FLAG_EDITED);
        // Representative 4; no full leaves, so no default and empty masks.
        assert_eq!(out[2], 4);
        assert_eq!(out[3..7], [0; 4]);
        // Two payloads after two masks, both inline.
        assert_eq!(out[11], PAYLOAD_UNIFORM_BIT | 4);
        assert_eq!(out[12], PAYLOAD_UNIFORM_BIT | 4);
        assert_eq!(out.len() % 4, 0);
        assert_eq!(e.words as usize, out.len());
    }

    #[test]
    fn mixed_leaves_get_a_palette_block() {
        let mut g = brick();
        g.set(VoxelPos::new(0, 0, 0), MaterialId(9));
        g.set(VoxelPos::new(1, 0, 0), MaterialId(3));
        g.set(VoxelPos::new(2, 0, 0), MaterialId(9));
        let mut out = Vec::new();
        encode_brick(g.root(), 0, false, &mut out, &mut Tally::default());
        let payload = out[9];
        assert_eq!(payload & PAYLOAD_UNIFORM_BIT, 0);
        let p = payload as usize;
        // Two materials: no header, the palette word first.
        assert_eq!(out[p], 3 | 9 << 16);
        // Indices of 9, 3, 9 at one bit each: 1, 0, 1.
        assert_eq!(out[p + 1], 0b101);
        assert_eq!((out[1] >> BLOCK_PALETTE_WORDS_SHIFT) & 0xFFFF, 2);
        // A third material brings the header back.
        g.set(VoxelPos::new(3, 0, 0), MaterialId(5));
        out.clear();
        encode_brick(g.root(), 0, false, &mut out, &mut Tally::default());
        let p = out[9] as usize;
        assert_eq!(out[p], 3 | 2 << PALETTE_BITS_SHIFT);
        assert_eq!(out[p + 1..p + 4], [3 | 5 << 16, 9, 2 | 2 << 4 | 1 << 6]);
    }

    #[test]
    fn full_leaves_are_a_default_mask_and_payloads() {
        let mut g = brick();
        // Leaves 0 and 2 full of 5, leaf 1 full of 6, leaf 3 one voxel of 5.
        for (leaf, m) in [(0, 5), (1, 6), (2, 5)] {
            fill_leaf(&mut g, leaf, m);
        }
        g.set(VoxelPos::new(12, 0, 0), MaterialId(5));
        let mut out = Vec::new();
        let e = encode_brick(g.root(), 0, false, &mut out, &mut Tally::default());
        assert_eq!(e.mask, 0b1111);
        assert_eq!(out[1] & BLOCK_COUNT_MASK, 1, "one partial leaf");
        assert_eq!(
            out[2],
            5 | 5 << BLOCK_DEFAULT_SHIFT,
            "representative, default"
        );
        // Leaf 3 is of the default too: in the default mask, not the full.
        let d = BLOCK_DEFAULT_MASK_WORD as usize;
        let f = BLOCK_FULL_MASK_WORD as usize;
        assert_eq!(out[d..d + 2], [0b1101, 0]);
        assert_eq!(out[f..f + 2], [0b111, 0]);
        // The partial leaf's mask, then the payload of leaf 1 alone.
        assert_eq!(out[7..9], [1, 0]);
        assert_eq!(out[9], PAYLOAD_UNIFORM_BIT | 6);
        assert_eq!(e.words, 12);
        assert_eq!(e.summary.solid, 3 * 64 + 1);
    }

    #[test]
    fn the_largest_block_fits_the_bound() {
        let mut g = brick();
        // 64 distinct materials in every leaf.
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    let slot = (x % 4) + 4 * (y % 4) + 16 * (z % 4);
                    g.set(VoxelPos::new(x, y, z), MaterialId(1 + slot as u16));
                }
            }
        }
        let mut out = Vec::new();
        let e = encode_brick(g.root(), 0, false, &mut out, &mut Tally::default());
        assert!(e.words <= MAX_BLOCK_WORDS.next_multiple_of(4));
        assert_eq!(
            e.words,
            (BLOCK_HEADER_WORDS + 64 * 3 + 64 * 49).next_multiple_of(4)
        );
    }
}
