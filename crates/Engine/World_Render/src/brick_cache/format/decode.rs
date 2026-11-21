//! A CPU decoder of the GPU format.
//!
//! It reads node and brick pools exactly as the traversal shader does, so
//! tests can check the encoder against the source voxel by voxel and check
//! read-back pools for consistency. It never panics on bad data: every
//! reference is bounds-checked and reported as a [`DecodeError`].

use std::fmt;

use world_core::MaterialId;
use world_core::coords::path::child_slot;

use super::{
    BLOCK_COUNT_MASK, BLOCK_DEFAULT_MASK_WORD, BLOCK_DEFAULT_SHIFT, BLOCK_DEFAULT_WORD,
    BLOCK_FULL_MASK_WORD, BLOCK_HEADER_WORDS, BrickRef, GpuNode, Kind, LEAF_MASK_WORDS,
    PALETTE_BITS_SHIFT, PALETTE_LEN_MASK, PALETTE_PAIR_SHIFT, PAYLOAD_UNIFORM_BIT, ROOT_NODE, rank,
};

/// The pools as the GPU holds them.
#[derive(Clone, Copy, Debug)]
pub struct PoolView<'a> {
    pub nodes: &'a [GpuNode],
    /// One word slice per brick segment.
    pub segments: &'a [&'a [u32]],
}

/// What the GPU format says about one voxel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    Air,
    /// Resident at full resolution.
    Exact(MaterialId),
    /// Only a coarse cell is resident: the voxel lies in a set mask cell of
    /// `4^level` voxels per axis drawn with `material`.
    Coarse {
        level: u32,
        material: MaterialId,
    },
}

/// A reference that does not point at valid data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    NodeOutOfRange {
        index: u32,
    },
    /// An internal record at level 1, or a brick record above level 1.
    KindAtWrongLevel {
        index: u32,
        level: u32,
    },
    BlockOutOfRange {
        block: BrickRef,
    },
    /// The block's masks or partial leaf count disagree with its node's
    /// mask.
    BlockCountMismatch {
        block: BrickRef,
    },
    PaletteOutOfRange {
        block: BrickRef,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for DecodeError {}

impl<'a> PoolView<'a> {
    fn node(&self, index: u32) -> Result<GpuNode, DecodeError> {
        self.nodes
            .get(index as usize)
            .copied()
            .ok_or(DecodeError::NodeOutOfRange { index })
    }

    /// The words of a block from its start to the end of its segment.
    pub fn block(&self, block: BrickRef) -> Result<&'a [u32], DecodeError> {
        let seg = self
            .segments
            .get(block.segment as usize)
            .ok_or(DecodeError::BlockOutOfRange { block })?;
        seg.get(block.word()..)
            .filter(|b| b.len() >= BLOCK_HEADER_WORDS as usize)
            .ok_or(DecodeError::BlockOutOfRange { block })
    }

    /// The voxel at world-local coordinate `local` of a tree whose root is
    /// a node of `root_level` (`levels - 1`).
    pub fn sample(&self, root_level: u32, local: [u32; 3]) -> Result<Sample, DecodeError> {
        let mut index = ROOT_NODE;
        let mut level = root_level;
        loop {
            let node = self.node(index)?;
            if node.kind() == Kind::Uniform {
                return Ok(Sample::Exact(node.material()));
            }
            let mask = node.mask();
            let slot = child_slot(local, level);
            if mask & (1u64 << slot) == 0 {
                return Ok(Sample::Air);
            }
            match node.kind() {
                Kind::NotResident => {
                    return Ok(Sample::Coarse {
                        level,
                        material: node.material(),
                    });
                }
                Kind::Internal if level >= 2 => {
                    index = node.reference.saturating_add(rank(mask, slot));
                    level -= 1;
                }
                Kind::Brick if level == 1 => {
                    let block = BrickRef::unpack(node.reference);
                    let words = self.block(block)?;
                    return block_voxel(words, block, mask, slot, child_slot(local, 0));
                }
                _ => return Err(DecodeError::KindAtWrongLevel { index, level }),
            }
        }
    }
}

/// The two-word mask at header word `at` of a block.
fn header_mask(words: &[u32], at: u32) -> u64 {
    u64::from(words[at as usize]) | u64::from(words[at as usize + 1]) << 32
}

/// Voxel `voxel_slot` of leaf `slot` of a block whose node has `mask`.
/// `words` holds at least the header.
pub fn block_voxel(
    words: &[u32],
    block: BrickRef,
    mask: u64,
    slot: u32,
    voxel_slot: u32,
) -> Result<Sample, DecodeError> {
    let default_mask = header_mask(words, BLOCK_DEFAULT_MASK_WORD);
    let full_mask = header_mask(words, BLOCK_FULL_MASK_WORD);
    let partial = mask & !full_mask;
    let paid = mask & !default_mask;
    let p = words[1] & BLOCK_COUNT_MASK;
    if (default_mask | full_mask) & !mask != 0
        || p != partial.count_ones()
        || mask & (1u64 << slot) == 0
    {
        return Err(DecodeError::BlockCountMismatch { block });
    }
    let bit = 1u64 << slot;
    let default = MaterialId((words[BLOCK_DEFAULT_WORD as usize] >> BLOCK_DEFAULT_SHIFT) as u16);
    if default_mask & full_mask & bit != 0 {
        return Ok(Sample::Exact(default));
    }
    let out = DecodeError::BlockOutOfRange { block };
    let leaf_mask = if full_mask & bit != 0 {
        u64::MAX
    } else {
        let at = (BLOCK_HEADER_WORDS + rank(partial, slot) * LEAF_MASK_WORDS) as usize;
        let lo = *words.get(at).ok_or(out)?;
        let hi = *words.get(at + 1).ok_or(out)?;
        u64::from(lo) | u64::from(hi) << 32
    };
    if leaf_mask & (1u64 << voxel_slot) == 0 {
        return Ok(Sample::Air);
    }
    if default_mask & bit != 0 {
        return Ok(Sample::Exact(default));
    }
    let payload = *words
        .get((BLOCK_HEADER_WORDS + p * LEAF_MASK_WORDS + rank(paid, slot)) as usize)
        .ok_or(out)?;
    if payload & PAYLOAD_UNIFORM_BIT != 0 {
        return Ok(Sample::Exact(MaterialId(payload as u16)));
    }
    let bad = DecodeError::PaletteOutOfRange { block };
    let p = payload as usize;
    let first = *words.get(p).ok_or(bad)?;
    // Two materials have no header: the first word is the palette.
    let (len, bits, palette_at) = if first >> PALETTE_PAIR_SHIFT != 0 {
        (2, 1, p)
    } else {
        (
            first & PALETTE_LEN_MASK,
            first >> PALETTE_BITS_SHIFT & 0xFF,
            p + 1,
        )
    };
    if !matches!(bits, 1 | 2 | 4 | 8) {
        return Err(bad);
    }
    let r = rank(leaf_mask, voxel_slot);
    let bit = r * bits;
    let indices_at = palette_at + len.div_ceil(2) as usize;
    let word = *words.get(indices_at + (bit / 32) as usize).ok_or(bad)?;
    let index = (word >> (bit % 32)) & ((1u32 << bits) - 1);
    if index >= len {
        return Err(bad);
    }
    let entry = *words.get(palette_at + (index / 2) as usize).ok_or(bad)?;
    Ok(Sample::Exact(MaterialId(
        (entry >> (16 * (index & 1))) as u16,
    )))
}
