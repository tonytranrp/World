//! The tree's storage layout: nodes, child words, leaf records and palette
//! blocks (decision 0004, with leaves stored inline in their parent).
//!
//! This layout is a format. Serialization reads it through the read API and
//! the GPU mirrors it word for word, so every fact below is asserted at
//! compile time or covered by tests, and changing it means matching WESL
//! changes.
//!
//! # Arenas
//!
//! A tree is a root child value plus two arenas addressed by `u32` index: an
//! array of [`Node`]s and an array of `u32` words. Snapshots expose both as
//! pages ([`NODE_PAGE_LEN`](super::NODE_PAGE_LEN) nodes and
//! [`WORD_PAGE_LEN`](super::WORD_PAGE_LEN) words each); a block of words
//! never straddles a page.
//!
//! # Slots
//!
//! A node at level `L` has 4x4x4 children, each `4^L` voxels per axis. For a
//! world-local coordinate (`pos - WORLD_MIN_VOXEL`), the child slot at level
//! `L` is `x + 4y + 16z`, where `x`, `y`, `z` are the two bits of each axis at
//! bit `2L`. Bit `i` of a 64-bit mask belongs to slot `i`, and the packed
//! entry of a set slot is found by its *rank*: the number of set mask bits
//! below it, `popcount(mask & ((1 << i) - 1))`.
//!
//! # Node: 16 bytes
//!
//! | Field  | Type  | Meaning                                              |
//! | ------ | ----- | ---------------------------------------------------- |
//! | `mask` | `u64` | bit `i` is set when child slot `i` is not all air    |
//! | `base` | `u32` | first word of the node's block                       |
//! | `info` | `u32` | level 1: stored leaf records; otherwise 0            |
//!
//! Every node is a branch of level 1 to `TREE_DEPTH - 1`; the root is level
//! `TREE_DEPTH - 1`. There are no leaf nodes: the 4x4x4-voxel leaves live in
//! the block of their level-1 parent. The level, known during traversal,
//! decides how the block is read. Live nodes have a non-zero mask.
//!
//! # Child words, level 2 and up: block of `n = popcount(mask)` words
//!
//! `words[base + rank(i)]` is the child in slot `i`:
//!
//! - bit 31 ([`UNIFORM_BIT`]) set: every voxel of the child is the material
//!   in the low 16 bits (never air); bits 16 to 30 are zero.
//! - exactly [`NOT_RESIDENT`] (bits 31 and 30): the child's content is not
//!   loaded (decision 0006). Only children at [`CHUNK_LEVEL`] and above can
//!   be not resident.
//! - bit 31 clear: the index of the child node, one level lower. The children
//!   of a level-5 node are the roots of chunks: their words are chunk ids.
//!
//! The root is a child word too, or air when the world is empty.
//!
//! # Chunks
//!
//! The subtree under a node at [`CHUNK_LEVEL`] (1024 voxels, 16 m, per axis)
//! is a *chunk* and lives in its own arenas, so it can be loaded, saved,
//! compacted and freed on its own (decision 0006). Levels above form a small
//! shared top. Node indices and word offsets inside a chunk refer to that
//! chunk's arenas.
//!
//! # Bricks, level 1: a header, then `3 * info` words
//!
//! A level-1 node covers one brick (16x16x16 voxels). Its block starts with a
//! [`BRICK_HEADER_WORDS`]-word header: a 64-bit *default mask* (two words, low
//! first) and a *default material*. A leaf whose slot is set in the default
//! mask is full and of the default material, and has no words at all; most
//! leaves under a terrain surface are such leaves. Every other non-air leaf
//! `i` is a record at `words[base + 3 + 3 * rank'(i)]`, where `rank'` counts
//! the set bits of `mask & !default_mask` below `i`, [`RECORD_WORDS`] words
//! each; `info` is how many there are. The default material is the most
//! common material among the brick's full single-material leaves (ties to
//! the lower id), and the default mask is exactly those leaves of it; a
//! brick without such leaves has a zero header.
//!
//! A record is:
//!
//! | Word | Meaning                                                    |
//! | ---- | ---------------------------------------------------------- |
//! | 0    | leaf mask, low 32 bits                                     |
//! | 1    | leaf mask, high 32 bits                                    |
//! | 2    | payload                                                    |
//!
//! Leaf mask bit `j` is set when voxel slot `j` of the leaf is solid; it is
//! never zero. The payload says what the solid voxels are:
//!
//! - bit 31 set: all of them are the material in the low 16 bits (bits 16 to
//!   30 zero). With a full mask the leaf is *uniform*: readers see it as a
//!   uniform child, exactly like a uniform child word.
//! - bit 31 clear: the word offset of the leaf's palette block; the leaf has
//!   two or more materials.
//!
//! # Palette block: `1 + ceil(p / 2) + ceil(s * b / 32)` words
//!
//! For a palette of `p` materials (2 to 64), `s = popcount(leaf mask)` solid
//! voxels and an index width of `b` bits:
//!
//! 1. Header: `p | b << 8`. `b` is the smallest of 1, 2, 4 or 8 that indexes
//!    `p` entries.
//! 2. Palette: the materials present, strictly ascending, two `u16` ids per
//!    word, low half first; an odd palette leaves the last high half zero.
//! 3. Indices: one per solid voxel in slot order, each the voxel's position
//!    in the palette, packed LSB-first into whole words. `b` divides 32, so
//!    an index never straddles a word; bits past the last index are zero.
//!
//! # Canonical form
//!
//! An all-air child is a clear mask bit with no word or record. A child whose
//! voxels are all one material is a uniform child word or a full-mask record
//! with that material, never a node or a palette. A palette holds exactly the
//! materials present, sorted, at the minimal width. So equal content has equal
//! masks, records, palettes and child words everywhere; only node indices and
//! word offsets may differ.

pub(crate) mod leaf;

use bytemuck::{Pod, Zeroable};

use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;

/// One branch node. `#[repr(C)]` with no padding, so the 16-byte layout is
/// fixed and node pages cast to bytes for the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Node {
    /// Bit `i` is set when child slot `i` is not all air.
    pub mask: u64,
    /// First word of the node's block in the word arena.
    pub base: u32,
    /// At level 1, the number of stored leaf records; 0 above. It also keeps
    /// the node padding-free at 16 bytes.
    pub info: u32,
}

const _: () = assert!(size_of::<Node>() == 16);
const _: () = assert!(align_of::<Node>() == 8);
const _: () = assert!(size_of::<Node>() == size_of::<u64>() + 2 * size_of::<u32>());
const _: () = assert!(TREE_FANOUT == u64::BITS);

/// Set in a child word or leaf payload that holds a material, not an index.
pub const UNIFORM_BIT: u32 = 1 << 31;

/// The child word of a child whose content is not loaded. It is not a valid
/// uniform word: a uniform word keeps bits 16 to 30 clear.
pub const NOT_RESIDENT: u32 = UNIFORM_BIT | 1 << 30;

/// The level of the nodes that root chunks. A chunk is 4^(CHUNK_LEVEL + 1)
/// = 1024 voxels per axis.
pub const CHUNK_LEVEL: u32 = 4;

/// Words in one leaf record of a level-1 node's block.
pub const RECORD_WORDS: usize = 3;

/// Words in the header of a level-1 node's block: the default mask and the
/// default material.
pub const BRICK_HEADER_WORDS: usize = 3;

/// The header of a level-1 node's block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BrickHeader {
    /// Leaves that are full and of `default`, with no record.
    pub full: u64,
    pub default: MaterialId,
}

impl BrickHeader {
    pub fn read(words: &[u32]) -> Self {
        Self {
            full: u64::from(words[0]) | u64::from(words[1]) << 32,
            default: MaterialId(words[2] as u16),
        }
    }

    pub const fn words(self) -> [u32; BRICK_HEADER_WORDS] {
        [
            self.full as u32,
            (self.full >> 32) as u32,
            self.default.0 as u32,
        ]
    }

    /// The word offset, from the block start, of leaf `slot`'s record in a
    /// node with `mask`. Only meaningful for a stored (non-default) leaf.
    pub const fn record_offset(self, mask: u64, slot: u32) -> usize {
        BRICK_HEADER_WORDS + RECORD_WORDS * rank(mask & !self.full, slot)
    }
}

/// Node indices are below this, so they never collide with uniform words.
pub(crate) const MAX_NODES: u32 = UNIFORM_BIT;

/// The longest node block: a level-1 node with a header and 64 records.
pub(crate) const MAX_BLOCK_LEN: usize = BRICK_HEADER_WORDS + RECORD_WORDS * TREE_FANOUT as usize;

impl Node {
    /// Words in the block of this node at `level`: one per child at level 2
    /// and up; the header and `info` records at level 1.
    pub(crate) const fn block_len(self, level: u32) -> usize {
        if level == 1 {
            BRICK_HEADER_WORDS + RECORD_WORDS * self.info as usize
        } else {
            self.mask.count_ones() as usize
        }
    }
}

/// One child of a level-2-or-higher node as the tree stores it. Air children
/// have no word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Child {
    /// Every voxel is air: a clear mask bit.
    #[default]
    Air,
    /// Every voxel is this solid material (never air).
    Uniform(MaterialId),
    /// A node index; the node's level is one below its parent's. Below a
    /// level-5 node it is a chunk id.
    Node(u32),
    /// The content is not loaded (only at [`CHUNK_LEVEL`] and above).
    NotResident,
}

impl Child {
    /// The child whose every voxel is `material`: air or uniform.
    pub const fn filled(material: MaterialId) -> Self {
        if material.is_air() {
            Self::Air
        } else {
            Self::Uniform(material)
        }
    }

    /// Decodes a stored child word. Stored words are never air.
    pub const fn from_word(word: u32) -> Self {
        if word == NOT_RESIDENT {
            Self::NotResident
        } else if word & UNIFORM_BIT != 0 {
            Self::Uniform(MaterialId(word as u16))
        } else {
            Self::Node(word)
        }
    }

    /// The word stored for this child, or `None` for air, which has no word.
    pub fn word(self) -> Option<u32> {
        match self {
            Self::Air => None,
            Self::Uniform(m) => {
                debug_assert!(!m.is_air(), "a uniform child is never air");
                Some(UNIFORM_BIT | u32::from(m.0))
            }
            Self::Node(index) => {
                debug_assert!(
                    index < MAX_NODES,
                    "node index {index} overlaps the uniform bit"
                );
                Some(index)
            }
            Self::NotResident => Some(NOT_RESIDENT),
        }
    }
}

/// The mask bit of child slot `slot` (0 to 63).
pub(crate) const fn bit(slot: u32) -> u64 {
    1 << slot
}

/// The position of slot `slot` among the set bits of `mask`: the number of
/// set bits below it. This is the index of the slot's entry in a packed block.
pub(crate) const fn rank(mask: u64, slot: u32) -> usize {
    (mask & (bit(slot) - 1)).count_ones() as usize
}

/// The set slots of `mask`, ascending.
pub(crate) fn slots(mask: u64) -> impl Iterator<Item = u32> + Clone {
    let mut rest = mask;
    std::iter::from_fn(move || {
        (rest != 0).then(|| {
            let slot = rest.trailing_zeros();
            rest &= rest - 1;
            slot
        })
    })
}

/// The child slot (0 to 63) that local coordinate `local` falls in at `level`.
/// Same as `coords::path::child_slot`, kept here so traversal inlines it.
#[inline]
pub(crate) const fn slot_at(local: [u32; 3], level: u32) -> u32 {
    let shift = 2 * level;
    ((local[0] >> shift) & 3) | (((local[1] >> shift) & 3) << 2) | (((local[2] >> shift) & 3) << 4)
}

/// The tree-order key of local coordinate `local` in a tree of `levels`
/// levels: the slots from the root down, six bits each, most significant
/// first. Sorting by it visits voxels in depth-first slot order, so voxels
/// that share a node are adjacent.
pub(crate) fn path_key(local: [u32; 3], levels: u32) -> u64 {
    (0..levels).fold(0, |key, level| {
        key | u64::from(slot_at(local, level)) << (6 * level)
    })
}

/// A node's content ready to store: its mask, its `info` and a block of up
/// to `N` words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackedNode<const N: usize> {
    pub mask: u64,
    pub info: u32,
    words: [u32; N],
    len: usize,
}

impl<const N: usize> PackedNode<N> {
    /// Panics if `words` is empty or longer than `N`.
    pub fn new(mask: u64, info: u32, words: &[u32]) -> Self {
        assert!(
            (1..=N).contains(&words.len()),
            "a node block holds 1 to {N} words, not {}",
            words.len()
        );
        let mut block = [0; N];
        block[..words.len()].copy_from_slice(words);
        Self {
            mask,
            info,
            words: block,
            len: words.len(),
        }
    }

    pub fn words(&self) -> &[u32] {
        &self.words[..self.len]
    }
}

/// What a rewritten node's content canonicalizes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "a short-lived stack value; boxing would allocate on every write"
)]
pub(crate) enum Packed<const N: usize> {
    /// Every voxel is this material (air included): no node is needed.
    Filled(MaterialId),
    /// Mixed content that needs a node.
    Node(PackedNode<N>),
    /// Every child is not resident: the node is not needed either.
    NotResident,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::path;

    #[test]
    fn child_words_round_trip() {
        for child in [
            Child::Uniform(MaterialId(1)),
            Child::Uniform(MaterialId(u16::MAX)),
            Child::Node(0),
            Child::Node(MAX_NODES - 1),
            Child::NotResident,
        ] {
            assert_eq!(Child::from_word(child.word().unwrap()), child);
        }
        assert_eq!(Child::Air.word(), None);
    }

    #[test]
    fn uniform_words_carry_the_material_in_the_low_half() {
        assert_eq!(Child::Uniform(MaterialId(0x1234)).word(), Some(0x8000_1234));
        assert_eq!(Child::Node(7).word(), Some(7));
    }

    #[test]
    fn filled_children_are_air_or_uniform() {
        assert_eq!(Child::filled(MaterialId::AIR), Child::Air);
        assert_eq!(Child::filled(MaterialId(3)), Child::Uniform(MaterialId(3)));
    }

    #[test]
    fn rank_counts_set_bits_below_the_slot() {
        let mask = 0b1011_0101u64 | bit(63);
        let expected = [0, 1, 1, 2, 2, 3, 4, 4, 5];
        for (slot, &want) in expected.iter().enumerate() {
            assert_eq!(rank(mask, slot as u32), want, "slot {slot}");
        }
        assert_eq!(rank(mask, 63), 5);
        assert_eq!(rank(u64::MAX, 63), 63);
        assert_eq!(rank(u64::MAX, 0), 0);
    }

    #[test]
    fn slots_lists_set_bits_in_order_and_ranks_match() {
        let mask = bit(0) | bit(5) | bit(17) | bit(63);
        let listed: Vec<u32> = slots(mask).collect();
        assert_eq!(listed, [0, 5, 17, 63]);
        for (k, slot) in slots(mask).enumerate() {
            assert_eq!(rank(mask, slot), k);
        }
        assert_eq!(slots(0).count(), 0);
        assert_eq!(slots(u64::MAX).count(), 64);
    }

    #[test]
    fn block_len_follows_the_level() {
        let node = Node {
            mask: bit(1) | bit(9) | bit(40),
            base: 0,
            info: 2,
        };
        assert_eq!(node.block_len(2), 3);
        assert_eq!(node.block_len(9), 3);
        assert_eq!(node.block_len(1), 9, "a header and two records");
        let full = Node {
            mask: u64::MAX,
            base: 0,
            info: 64,
        };
        assert_eq!(full.block_len(1), MAX_BLOCK_LEN);
    }

    #[test]
    fn brick_headers_round_trip_and_skip_default_leaves() {
        let h = BrickHeader {
            full: bit(2) | bit(63),
            default: MaterialId(7),
        };
        assert_eq!(BrickHeader::read(&h.words()), h);
        let mask = bit(0) | bit(2) | bit(5) | bit(63);
        assert_eq!(h.record_offset(mask, 0), 3);
        assert_eq!(h.record_offset(mask, 5), 6, "slot 2 has no record");
    }

    #[test]
    fn slot_at_matches_the_coordinate_path() {
        for local in [[0, 0, 0], [5, 900, 77_777], [1 << 19, (1 << 20) - 1, 3]] {
            for level in 0..10 {
                assert_eq!(slot_at(local, level), path::child_slot(local, level));
            }
        }
    }

    #[test]
    fn path_keys_order_voxels_depth_first() {
        // The key's top six bits are the root slot, the lowest the voxel slot.
        assert_eq!(path_key([1, 0, 0], 2), 1);
        assert_eq!(path_key([4, 0, 0], 2), 1 << 6);
        assert_eq!(path_key([0, 0, 4 * 3 + 3], 2), (48 << 6) | 48);
        // Every voxel of one leaf sorts before any voxel of the next leaf.
        let leaf_last = path_key([3, 3, 3], 3);
        let next_leaf_first = path_key([4, 0, 0], 3);
        assert!(leaf_last < next_leaf_first);
        // Keys are a bijection on a small cube.
        let mut keys: Vec<u64> = (0..16 * 16 * 16)
            .map(|i| path_key([i % 16, i / 16 % 16, i / 256], 2))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 4096);
        assert_eq!(keys.last(), Some(&4095));
    }

    #[test]
    #[should_panic(expected = "1 to 64 words")]
    fn packed_nodes_are_never_empty() {
        PackedNode::<64>::new(0, 0, &[]);
    }
}
