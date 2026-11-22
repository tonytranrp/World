//! The GPU tree format of decision 0007.
//!
//! # Node record, 16 bytes, 16-byte aligned
//!
//! | word | content |
//! |------|---------|
//! | 0 | `mask_lo`: occupancy bits 0..31 of the node's 64 children |
//! | 1 | `mask_hi`: occupancy bits 32..63 |
//! | 2 | `ref`: depends on the kind, see below |
//! | 3 | `info`: bits 0..15 representative material, 16..17 kind, 18 subtree contains edits, 19..31 reserved (zero) |
//!
//! Kinds:
//! - [`KIND_INTERNAL`]: the children are records too; `ref` is the index of
//!   the first one, children are contiguous, and the child in `slot` is at
//!   `ref + popcount(mask & below(slot))`. One dependent load per level.
//! - [`KIND_BRICK`] (level-1 nodes only): `ref` bits 0..27 are the brick
//!   block's offset in 16-byte units inside segment `ref >> 28`.
//! - [`KIND_NOT_RESIDENT`]: the children (or the brick) are not on the GPU;
//!   the mask and the material are still drawable at this node's cell size.
//! - [`KIND_UNIFORM`]: every voxel of the node is one material, kept in `ref`
//!   bits 0..15 and in the info material; the mask is all ones.
//!
//! A record describes a node of level 1 to `levels - 1` (the root, at index
//! [`ROOT_NODE`]). Air children have no record.
//!
//! # Brick block, one per resident level-1 node, 16-byte aligned
//!
//! | words | content |
//! |-------|---------|
//! | 0 | brick id (index into the brick table) |
//! | 1 | `p \| palette_words << 8 \| flags << 24` |
//! | 2 | representative material in bits 0..15, default material in bits 16..31: the most common material of the brick's full one-material leaves, ties to the lower id; air when it has none |
//! | 3, 4 | default mask `A`: the leaves whose solid voxels are all the default material, full or not; they have no payload |
//! | 5, 6 | full mask `B`: every leaf that is full of one material; it has no leaf mask |
//! | 7 .. 7+2p | `leaf_mask[p]`: lo, hi word per partial leaf (`P = mask & !B`), in slot order |
//! | 7+2p .. 7+2p+s | `payload[s]` per leaf of `S = mask & !A`, in slot order: bit 31 set: the leaf's solid voxels are all material `payload & 0xFFFF`; else the word offset from the block start to the leaf's palette block |
//! | 7+2p+s .. | palette blocks (`palette_words` words), then zero padding to 16 bytes |
//!
//! So leaf `i` of a brick is the default material throughout when both
//! masks have it; else its leaf mask is all ones when `B` has it and
//! `leaf_mask[rank(P, i)]` otherwise, and its solid voxels are the default
//! material when `A` has it and `payload[rank(S, i)]` says otherwise
//! (decisions 0021 and 0025).
//!
//! A palette block (decisions 0005 and 0025): for three or more materials a
//! header `p | b << 8` (`p` materials, `b` bits per index, the smallest of
//! 1, 2, 4, 8 with `2^b >= p`), then the palette ascending, two u16 per word
//! low half first, then one index per solid voxel in slot order, `b` bits
//! each, LSB-first in whole words. A block of two materials has no header:
//! it starts at its palette word, whose high half (the larger id) is never
//! zero, where a header's is ([`PALETTE_PAIR_SHIFT`]).
//!
//! # Brick table, 8 bytes per brick id
//!
//! `[block ref, last-used frame]`; a free id holds [`INVALID_REF`].

pub mod brick;
pub mod decode;
pub mod summary;

use bytemuck::{Pod, Zeroable};
use world_core::MaterialId;

/// Words in one node record.
pub const NODE_WORDS: u32 = 4;
/// Bytes in one node record.
pub const NODE_BYTES: u64 = 16;
/// Index of the root's record in the node pool.
pub const ROOT_NODE: u32 = 0;

/// Info bits holding the representative material.
pub const INFO_MATERIAL_MASK: u32 = 0xFFFF;
/// Shift of the two kind bits in the info word.
pub const INFO_KIND_SHIFT: u32 = 16;
/// Mask of the kind after shifting.
pub const INFO_KIND_MASK: u32 = 0x3;
/// Info bit: the node's subtree contains edited bricks.
pub const INFO_EDITED_BIT: u32 = 1 << 18;

pub const KIND_INTERNAL: u32 = 0;
pub const KIND_BRICK: u32 = 1;
pub const KIND_NOT_RESIDENT: u32 = 2;
pub const KIND_UNIFORM: u32 = 3;

/// Bits of a brick reference holding the offset in 16-byte units.
pub const BRICK_REF_OFFSET_MASK: u32 = (1 << 28) - 1;
/// Shift of the segment number in a brick reference.
pub const BRICK_REF_SEGMENT_SHIFT: u32 = 28;
/// Brick pool segments the shaders bind. Unused ones get a dummy buffer.
pub const MAX_SEGMENTS: u32 = 4;
/// Brick blocks are aligned to this many words (16 bytes).
pub const BLOCK_ALIGN_WORDS: u32 = 4;

/// Words in a brick block header.
pub const BLOCK_HEADER_WORDS: u32 = 7;
/// Header word holding the representative material (low half) and the
/// default material (high half).
pub const BLOCK_DEFAULT_WORD: u32 = 2;
/// Shift of the default material in its header word.
pub const BLOCK_DEFAULT_SHIFT: u32 = 16;
/// First of the two header words of the default mask (low word first).
pub const BLOCK_DEFAULT_MASK_WORD: u32 = 3;
/// First of the two header words of the full mask (low word first).
pub const BLOCK_FULL_MASK_WORD: u32 = 5;
/// Header word 1: mask of the partial leaf count.
pub const BLOCK_COUNT_MASK: u32 = 0xFF;
/// Header word 1: shift of the palette word count.
pub const BLOCK_PALETTE_WORDS_SHIFT: u32 = 8;
/// Header word 1: mask of the palette word count after shifting.
pub const BLOCK_PALETTE_WORDS_MASK: u32 = 0xFFFF;
/// Header word 1: shift of the flags byte.
pub const BLOCK_FLAGS_SHIFT: u32 = 24;
/// Block flag: the brick has been edited this session.
pub const BLOCK_FLAG_EDITED: u32 = 1;
/// Words per leaf mask.
pub const LEAF_MASK_WORDS: u32 = 2;
/// Payload bit: the leaf's solid voxels are one material.
pub const PAYLOAD_UNIFORM_BIT: u32 = 1 << 31;
/// Palette header: mask of the palette length.
pub const PALETTE_LEN_MASK: u32 = 0xFF;
/// Palette header: shift of the index width.
pub const PALETTE_BITS_SHIFT: u32 = 8;
/// A palette block whose first word has bits at or above this shift set is
/// a two-material block without a header: the word is its palette, the
/// larger id in the high half (decision 0025).
pub const PALETTE_PAIR_SHIFT: u32 = 16;

/// Words per brick table entry: block ref, last-used frame.
pub const BRICK_TABLE_WORDS: u32 = 2;
/// A free brick id's block ref, and "none" in general.
pub const INVALID_REF: u32 = u32::MAX;

/// The kind of a node record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Internal,
    Brick,
    NotResident,
    Uniform,
}

impl Kind {
    pub const fn bits(self) -> u32 {
        match self {
            Self::Internal => KIND_INTERNAL,
            Self::Brick => KIND_BRICK,
            Self::NotResident => KIND_NOT_RESIDENT,
            Self::Uniform => KIND_UNIFORM,
        }
    }

    pub const fn from_bits(bits: u32) -> Self {
        match bits & INFO_KIND_MASK {
            KIND_INTERNAL => Self::Internal,
            KIND_BRICK => Self::Brick,
            KIND_NOT_RESIDENT => Self::NotResident,
            _ => Self::Uniform,
        }
    }
}

/// Where a brick block lives: segment and offset in 16-byte units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BrickRef {
    pub segment: u32,
    pub offset16: u32,
}

impl BrickRef {
    /// Packs into a `ref` word. The caller keeps the segment below
    /// [`MAX_SEGMENTS`] and the offset within 28 bits (the pool guarantees it).
    pub const fn pack(self) -> u32 {
        (self.segment << BRICK_REF_SEGMENT_SHIFT) | (self.offset16 & BRICK_REF_OFFSET_MASK)
    }

    pub const fn unpack(word: u32) -> Self {
        Self {
            segment: word >> BRICK_REF_SEGMENT_SHIFT,
            offset16: word & BRICK_REF_OFFSET_MASK,
        }
    }

    /// Word index of the block start inside its segment.
    pub const fn word(self) -> usize {
        self.offset16 as usize * BLOCK_ALIGN_WORDS as usize
    }
}

/// One node record exactly as the GPU reads it.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct GpuNode {
    pub mask_lo: u32,
    pub mask_hi: u32,
    pub reference: u32,
    pub info: u32,
}

const _: () = assert!(size_of::<GpuNode>() as u64 == NODE_BYTES);

fn info(kind: Kind, material: MaterialId, edited: bool) -> u32 {
    u32::from(material.0)
        | kind.bits() << INFO_KIND_SHIFT
        | if edited { INFO_EDITED_BIT } else { 0 }
}

impl GpuNode {
    fn with(mask: u64, reference: u32, info: u32) -> Self {
        Self {
            mask_lo: mask as u32,
            mask_hi: (mask >> 32) as u32,
            reference,
            info,
        }
    }

    /// A node whose children are records starting at `first_child`.
    pub fn internal(mask: u64, first_child: u32, material: MaterialId, edited: bool) -> Self {
        Self::with(mask, first_child, info(Kind::Internal, material, edited))
    }

    /// A level-1 node whose brick block is resident at `block`.
    pub fn brick(mask: u64, block: BrickRef, material: MaterialId, edited: bool) -> Self {
        Self::with(mask, block.pack(), info(Kind::Brick, material, edited))
    }

    /// A node whose children are not on the GPU; drawable from mask and material.
    pub fn not_resident(mask: u64, material: MaterialId, edited: bool) -> Self {
        Self::with(mask, INVALID_REF, info(Kind::NotResident, material, edited))
    }

    /// A node that is one material throughout.
    pub fn uniform(material: MaterialId, edited: bool) -> Self {
        Self::with(
            u64::MAX,
            u32::from(material.0),
            info(Kind::Uniform, material, edited),
        )
    }

    pub fn mask(&self) -> u64 {
        u64::from(self.mask_lo) | u64::from(self.mask_hi) << 32
    }

    pub fn kind(&self) -> Kind {
        Kind::from_bits(self.info >> INFO_KIND_SHIFT)
    }

    pub fn material(&self) -> MaterialId {
        MaterialId((self.info & INFO_MATERIAL_MASK) as u16)
    }

    pub fn edited(&self) -> bool {
        self.info & INFO_EDITED_BIT != 0
    }

    /// The same record with kind not resident, keeping mask and material:
    /// what unlinking an evicted brick writes.
    pub fn unlinked(&self) -> Self {
        Self::not_resident(self.mask(), self.material(), self.edited())
    }
}

/// Number of set bits of `mask` below `slot` (0 to 63): the child's rank.
pub const fn rank(mask: u64, slot: u32) -> u32 {
    (mask & ((1u64 << slot) - 1)).count_ones()
}

/// Every format constant with its name, as `cargo xtask codegen` writes it
/// to `assets/shaders/common/constants.wesl`.
pub const SHADER_CONSTANTS: &[(&str, u32)] = &[
    ("NODE_WORDS", NODE_WORDS),
    ("ROOT_NODE", ROOT_NODE),
    ("INFO_MATERIAL_MASK", INFO_MATERIAL_MASK),
    ("INFO_KIND_SHIFT", INFO_KIND_SHIFT),
    ("INFO_KIND_MASK", INFO_KIND_MASK),
    ("INFO_EDITED_BIT", INFO_EDITED_BIT),
    ("KIND_INTERNAL", KIND_INTERNAL),
    ("KIND_BRICK", KIND_BRICK),
    ("KIND_NOT_RESIDENT", KIND_NOT_RESIDENT),
    ("KIND_UNIFORM", KIND_UNIFORM),
    ("BRICK_REF_OFFSET_MASK", BRICK_REF_OFFSET_MASK),
    ("BRICK_REF_SEGMENT_SHIFT", BRICK_REF_SEGMENT_SHIFT),
    ("MAX_SEGMENTS", MAX_SEGMENTS),
    ("BLOCK_ALIGN_WORDS", BLOCK_ALIGN_WORDS),
    ("BLOCK_HEADER_WORDS", BLOCK_HEADER_WORDS),
    ("BLOCK_DEFAULT_WORD", BLOCK_DEFAULT_WORD),
    ("BLOCK_DEFAULT_SHIFT", BLOCK_DEFAULT_SHIFT),
    ("BLOCK_DEFAULT_MASK_WORD", BLOCK_DEFAULT_MASK_WORD),
    ("BLOCK_FULL_MASK_WORD", BLOCK_FULL_MASK_WORD),
    ("BLOCK_COUNT_MASK", BLOCK_COUNT_MASK),
    ("BLOCK_PALETTE_WORDS_SHIFT", BLOCK_PALETTE_WORDS_SHIFT),
    ("BLOCK_PALETTE_WORDS_MASK", BLOCK_PALETTE_WORDS_MASK),
    ("BLOCK_FLAGS_SHIFT", BLOCK_FLAGS_SHIFT),
    ("BLOCK_FLAG_EDITED", BLOCK_FLAG_EDITED),
    ("LEAF_MASK_WORDS", LEAF_MASK_WORDS),
    ("PAYLOAD_UNIFORM_BIT", PAYLOAD_UNIFORM_BIT),
    ("PALETTE_LEN_MASK", PALETTE_LEN_MASK),
    ("PALETTE_BITS_SHIFT", PALETTE_BITS_SHIFT),
    ("PALETTE_PAIR_SHIFT", PALETTE_PAIR_SHIFT),
    ("BRICK_TABLE_WORDS", BRICK_TABLE_WORDS),
    ("INVALID_REF", INVALID_REF),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_their_fields() {
        let mask = 0x8000_0001_0000_0003;
        let n = GpuNode::internal(mask, 42, MaterialId(7), true);
        assert_eq!(
            (n.mask(), n.kind(), n.material(), n.edited()),
            (mask, Kind::Internal, MaterialId(7), true)
        );
        assert_eq!(n.reference, 42);

        let block = BrickRef {
            segment: 3,
            offset16: BRICK_REF_OFFSET_MASK,
        };
        let b = GpuNode::brick(mask, block, MaterialId(9), false);
        assert_eq!(b.kind(), Kind::Brick);
        assert_eq!(BrickRef::unpack(b.reference), block);

        let u = GpuNode::uniform(MaterialId(5), false);
        assert_eq!(
            (u.mask(), u.reference, u.kind()),
            (u64::MAX, 5, Kind::Uniform)
        );

        let gone = b.unlinked();
        assert_eq!(
            (gone.kind(), gone.mask(), gone.material()),
            (Kind::NotResident, mask, MaterialId(9))
        );
    }

    #[test]
    fn rank_counts_bits_below_the_slot() {
        assert_eq!(rank(0b1011, 0), 0);
        assert_eq!(rank(0b1011, 3), 2);
        assert_eq!(rank(u64::MAX, 63), 63);
    }

    #[test]
    fn nodes_are_sixteen_bytes_aligned_to_sixteen() {
        assert_eq!(align_of::<GpuNode>(), 16);
        assert_eq!(size_of::<GpuNode>(), 16);
    }
}
