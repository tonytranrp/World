//! The brick block encoder: one level-1 node into its GPU block.

use world_core::MaterialId;
use world_core::tree::NodeView;

use super::summary::{Content, Summary, Tally, content};
use super::{
    BLOCK_ALIGN_WORDS, BLOCK_FLAG_EDITED, BLOCK_FLAGS_SHIFT, BLOCK_HEADER_WORDS,
    BLOCK_PALETTE_WORDS_SHIFT, LEAF_MASK_WORDS, PALETTE_BITS_SHIFT, PAYLOAD_UNIFORM_BIT,
};

/// The largest block a brick can need, in words: header, 64 leaves with a
/// mask and payload each, and 64 palette blocks of up to 64 materials with
/// 8-bit indices (1 + 32 + 16 words).
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

/// Words of a palette block for `len` materials and `solid` voxels.
pub const fn palette_block_words(len: u32, solid: u32) -> u32 {
    1 + len.div_ceil(2) + (solid * index_bits(len)).div_ceil(32)
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
/// once; the representative material is counted from the same reads.
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
    let n = mask.count_ones();
    scratch.clear();

    // Header, then room for masks and payloads, filled below.
    out.extend_from_slice(&[brick_id, 0, 0, 0]);
    out.resize(
        start + (BLOCK_HEADER_WORDS + n * (LEAF_MASK_WORDS + 1)) as usize,
        0,
    );
    let masks_at = start + BLOCK_HEADER_WORDS as usize;
    let payloads_at = masks_at + (n * LEAF_MASK_WORDS) as usize;

    for (k, (_, leaf)) in node.children().enumerate() {
        let leaf_mask = leaf.mask();
        out[masks_at + 2 * k] = leaf_mask as u32;
        out[masks_at + 2 * k + 1] = (leaf_mask >> 32) as u32;
        out[payloads_at + k] = match content(&leaf) {
            Content::Uniform(m) => {
                scratch.add(m, 64);
                PAYLOAD_UNIFORM_BIT | u32::from(m.0)
            }
            // Most surface leaves hold one material: the stored palette says
            // so without reading voxels (the world tree keeps it exact).
            _ if let [m] = leaf.palette().as_slice() => {
                scratch.add(*m, u64::from(leaf_mask.count_ones()));
                PAYLOAD_UNIFORM_BIT | u32::from(m.0)
            }
            _ => {
                let scan = LeafScan::of(&leaf, leaf_mask);
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
        };
    }

    let summary = scratch.summary();
    out[start + 2] = u32::from(summary.material.0);
    let palette_words = (out.len() - payloads_at - n as usize) as u32;
    let flags = if edited { BLOCK_FLAG_EDITED } else { 0 };
    out[start + 1] = n | palette_words << BLOCK_PALETTE_WORDS_SHIFT | flags << BLOCK_FLAGS_SHIFT;

    let padded = (out.len() - start).next_multiple_of(BLOCK_ALIGN_WORDS as usize);
    out.resize(start + padded, 0);
    EncodedBrick {
        mask,
        summary,
        words: padded as u32,
    }
}

/// Appends one leaf's palette block: header, palette, packed indices.
fn append_palette_block(scan: &LeafScan, out: &mut Vec<u32>) {
    let palette = &scan.palette[..scan.len];
    let len = scan.len as u32;
    let bits = index_bits(len);
    out.push(len | bits << PALETTE_BITS_SHIFT);
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
    use world_core::{DenseGrid, VoxelPos, VoxelSource};

    fn brick() -> DenseGrid {
        DenseGrid::new(2, VoxelPos::new(0, 0, 0))
    }

    #[test]
    fn index_widths_follow_the_palette_length() {
        assert_eq!(
            [2, 3, 4, 5, 16, 17, 64].map(index_bits),
            [1, 2, 2, 4, 4, 8, 8]
        );
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
        assert_eq!(out[2], 4);
        // Two payloads after two masks, both inline.
        assert_eq!(out[8], PAYLOAD_UNIFORM_BIT | 4);
        assert_eq!(out[9], PAYLOAD_UNIFORM_BIT | 4);
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
        let payload = out[6];
        assert_eq!(payload & PAYLOAD_UNIFORM_BIT, 0);
        let p = payload as usize;
        assert_eq!(out[p], 2 | 1 << PALETTE_BITS_SHIFT);
        assert_eq!(out[p + 1], 3 | 9 << 16);
        // Indices of 9, 3, 9 at one bit each: 1, 0, 1.
        assert_eq!(out[p + 2], 0b101);
        assert_eq!((out[1] >> BLOCK_PALETTE_WORDS_SHIFT) & 0xFFFF, 3);
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
        assert!(e.words <= MAX_BLOCK_WORDS);
        assert_eq!(e.words, MAX_BLOCK_WORDS.next_multiple_of(4));
    }
}
