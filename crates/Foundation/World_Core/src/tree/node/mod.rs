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
//! | `info` | `u32` | level 1: block length and flags; otherwise 0           |
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
//! # Bricks, level 1: a header, then the stored leaves (decisions 0021, 0025, 0031)
//!
//! A level-1 node covers one brick (16x16x16 voxels). Its `info` holds the
//! block's length in words (bits 0 to 8, [`BRICK_LEN_MASK`]),
//! [`BRICK_FULL_MASK`] (bit 9) and [`BRICK_PAIR_MASK`] (bit 10). Its block
//! starts with a header:
//!
//! | Word | Meaning                                                    |
//! | ---- | ---------------------------------------------------------- |
//! | 0, 1 | *default mask* `A`, low word first                         |
//! | 2    | *default material*                                         |
//! | next 2 | *full mask* `B`, low word first; only with `BRICK_FULL_MASK` |
//! | next 2 | *pair mask* `P`, low word first; only with `BRICK_PAIR_MASK` |
//!
//! The default material is the most common material among the brick's full
//! single-material leaves (ties to the lower id); a brick without such
//! leaves has a zero default mask and material. The default mask is every
//! leaf whose solid voxels are all the default material: the full ones and
//! the *default partial* ones. The full mask is every full single-material
//! leaf.
//!
//! The header stores the full mask only when it differs from the default
//! mask, that is when the brick has a full leaf of another material or a
//! default partial leaf, and the *mixed mask* `P` (decision 0031 called it
//! the pair mask) only when it is not zero: the leaves of two or more
//! materials. Without either the header is three words and every stored
//! leaf is a record, as before decision 0025.
//!
//! After the header comes every non-air leaf outside `B`, in slot order, by
//! its mask bits:
//!
//! | In `A` | In `B` | In `P` | Leaf                     | Words                     |
//! | ------ | ------ | ------ | ------------------------ | ------------------------- |
//! | yes    | yes    | no     | full of the default      | none                      |
//! | no     | yes    | no     | full of another material | none here: a half below   |
//! | yes    | no     | no     | default partial          | its leaf mask, lo and hi  |
//! | no     | no     | yes    | two or more materials    | [`PAIR_WORDS`]: leaf mask, then three words below |
//! | no     | no     | no     | one material and air     | a [`RECORD_WORDS`]-word record |
//!
//! Last come the materials of the full leaves of other materials, `O = B &
//! !A`, as 16-bit halves in slot order, two a word, low half first
//! (`ceil(popcount(O) / 2)` words ending the block, an odd count leaving the
//! last high half zero; decisions 0033 and 0035).
//!
//! So a leaf outside `B` is `2 + [not in A] + 2 * [in P]` words, stored leaf
//! `i` starts at `base + header + 2 * rank(mask & !B, i) + rank(mask & !A &
//! !B, i) + 2 * rank(P, i)`, and the material of a leaf in `O` is half
//! `rank(O, i)` of the block's last `ceil(popcount(O) / 2)` words.
//!
//! The three words after a mixed leaf's mask are, for two materials, its
//! palette block exactly as a block stored apart would start (the palette
//! word, then the indices, one bit per solid voxel; the second index word is
//! zero when it has 32 solid voxels or fewer; decision 0031). For three or
//! more materials they are the word offset of its palette block, stored
//! apart, then two zero words (decision 0035): a two-material leaf's indices
//! are never zero, since both materials are present, so the zero words tell
//! them apart, and a leaf that gains or loses a material keeps its length.
//!
//! A record is:
//!
//! | Word | Meaning                                                    |
//! | ---- | ---------------------------------------------------------- |
//! | 0    | leaf mask, low 32 bits                                     |
//! | 1    | leaf mask, high 32 bits                                    |
//! | 2    | [`UNIFORM_BIT`] and the material in the low 16 bits        |
//!
//! Leaf mask bit `j` is set when voxel slot `j` of the leaf is solid; it is
//! never zero, and never all ones with a single material (such a leaf is
//! full and has no record).
//!
//! # Palette block
//!
//! Leaves of three or more materials keep their palette block apart in the
//! word arena; a two-material leaf's block is inline in its brick (above),
//! laid out the same way. For a palette of `p` materials (2 to 64), `s =
//! popcount(leaf mask)` solid
//! voxels and an index width of `b` bits, the smallest of 1, 2, 4 or 8 that
//! indexes `p` entries:
//!
//! 1. Header: `p | b << 8`, for three or more materials only (decision
//!    0025). A two-material block has no header and starts at the palette.
//! 2. Palette: the materials present, strictly ascending, two `u16` ids per
//!    word, low half first; an odd palette leaves the last high half zero.
//! 3. Indices: one per solid voxel in slot order, each the voxel's position
//!    in the palette, packed LSB-first into whole words. `b` divides 32, so
//!    an index never straddles a word; bits past the last index are zero.
//!
//! The first word tells the two apart: a header's high half is zero, while a
//! two-material palette's high half is its larger id, which is at least 2.
//! A block is `1 + ceil(s / 32)` words for two materials and
//! `1 + ceil(p / 2) + ceil(s * b / 32)` otherwise.
//!
//! # Canonical form
//!
//! An all-air child is a clear mask bit with no word or record. A child whose
//! voxels are all one material is a uniform child word, a default leaf or an
//! other-full word, never a node, a record or a palette. A partial leaf of
//! one material is a record only when that material is not the default. A
//! leaf of two or more materials is always in the mixed mask, inline when it
//! has two. A palette holds exactly the
//! materials present, sorted, at the minimal width. So equal content has equal
//! masks, records, palettes and child words everywhere; only node indices and
//! word offsets may differ.

pub(crate) mod leaf;

use bytemuck::{Pod, Zeroable};

use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;
use leaf::Record;

/// One branch node. `#[repr(C)]` with no padding, so the 16-byte layout is
/// fixed and node pages cast to bytes for the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Node {
    /// Bit `i` is set when child slot `i` is not all air.
    pub mask: u64,
    /// First word of the node's block in the word arena.
    pub base: u32,
    /// At level 1, the block length and [`BRICK_FULL_MASK`]; 0 above. It
    /// also keeps the node padding-free at 16 bytes.
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

/// Words in the header of a level-1 node's block without a full mask: the
/// default mask and the default material.
pub const BRICK_HEADER_WORDS: usize = 3;

/// Words of a leaf of two or more materials (decisions 0031 and 0035): its
/// leaf mask, then a two-material leaf's palette word and two index words,
/// or a wider palette's block offset and two zero words.
pub const PAIR_WORDS: usize = 5;

/// Words in the header of a level-1 node's block with both a full mask and
/// a pair mask.
pub const BRICK_HEADER_MAX_WORDS: usize = BRICK_HEADER_WORDS + 4;

/// Level-1 `info` bits holding the block's length in words.
pub const BRICK_LEN_MASK: u32 = 0x1FF;

/// Level-1 `info` flag: the header has a full mask (decision 0025; 0021
/// called it the other-full flag).
pub const BRICK_FULL_MASK: u32 = 1 << 9;

/// Level-1 `info` flag: the header has a mixed mask (decision 0031, which
/// called it the pair mask; 0035 widened it to every leaf of two or more
/// materials).
pub const BRICK_PAIR_MASK: u32 = 1 << 10;

/// The header of a level-1 node's block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BrickHeader {
    /// Leaves whose solid voxels are all `default`: the full ones have no
    /// words, the partial ones a leaf mask.
    pub defaults: u64,
    pub default: MaterialId,
    /// Leaves full of one material: those in `defaults` have no words, the
    /// others one word. Equal to `defaults` when the header has no full mask.
    pub full: u64,
    /// The mixed mask: leaves of two or more materials, [`PAIR_WORDS`]
    /// words each.
    pub pairs: u64,
}

impl BrickHeader {
    /// The header of a brick whose leaves stored as defaults are `defaults`,
    /// all full: no full mask, no pairs.
    #[cfg(test)]
    pub const fn short(defaults: u64, default: MaterialId) -> Self {
        Self {
            defaults,
            default,
            full: defaults,
            pairs: 0,
        }
    }

    /// Reads the header of a block whose node has level-1 `info`. `words`
    /// holds at least the header.
    #[inline]
    pub fn read(words: &[u32], info: u32) -> Self {
        let defaults = u64::from(words[0]) | u64::from(words[1]) << 32;
        let (full, next) = if info & BRICK_FULL_MASK != 0 {
            (u64::from(words[3]) | u64::from(words[4]) << 32, 5)
        } else {
            (defaults, 3)
        };
        Self {
            defaults,
            default: MaterialId(words[2] as u16),
            full,
            pairs: if info & BRICK_PAIR_MASK != 0 {
                u64::from(words[next]) | u64::from(words[next + 1]) << 32
            } else {
                0
            },
        }
    }

    /// Whether the header stores the full mask.
    pub const fn has_full_mask(self) -> bool {
        self.full != self.defaults
    }

    /// Words in this header: 3, 2 more with a full mask, 2 more with pairs.
    pub const fn len(self) -> usize {
        BRICK_HEADER_WORDS
            + if self.has_full_mask() { 2 } else { 0 }
            + if self.pairs != 0 { 2 } else { 0 }
    }

    /// The header's words, of which the first [`len`](Self::len) are stored.
    pub const fn words(self) -> [u32; BRICK_HEADER_MAX_WORDS] {
        let mut words = [0; BRICK_HEADER_MAX_WORDS];
        words[0] = self.defaults as u32;
        words[1] = (self.defaults >> 32) as u32;
        words[2] = self.default.0 as u32;
        let mut at = BRICK_HEADER_WORDS;
        if self.has_full_mask() {
            words[at] = self.full as u32;
            words[at + 1] = (self.full >> 32) as u32;
            at += 2;
        }
        if self.pairs != 0 {
            words[at] = self.pairs as u32;
            words[at + 1] = (self.pairs >> 32) as u32;
        }
        words
    }

    /// The level-1 `info` of a block with this header and `len` words.
    pub const fn info(self, len: usize) -> u32 {
        len as u32
            | if self.has_full_mask() {
                BRICK_FULL_MASK
            } else {
                0
            }
            | if self.pairs != 0 { BRICK_PAIR_MASK } else { 0 }
    }

    /// The full leaves of the default material: no words.
    pub const fn default_full(self) -> u64 {
        self.defaults & self.full
    }

    /// The full leaves of other materials: a 16-bit half each.
    pub const fn other_full(self) -> u64 {
        self.full & !self.defaults
    }

    /// Words of the other full leaves' halves, which end the block.
    pub const fn half_words(self) -> usize {
        (self.other_full().count_ones() as usize).div_ceil(2)
    }

    /// The word offset, from the block start, where the leaves outside the
    /// full mask start: right after the header.
    pub const fn leaves_start(self) -> usize {
        self.len()
    }

    /// The material of other full leaf `slot` (set in
    /// [`other_full`](Self::other_full)), read from `block`, the node's
    /// whole block.
    #[inline]
    pub fn half(self, block: &[u32], slot: u32) -> MaterialId {
        let k = rank(self.other_full(), slot);
        let at = block.len() - self.half_words() + k / 2;
        MaterialId((block[at] >> (16 * (k % 2))) as u16)
    }

    /// The word offset, from the block start, of the stored words of leaf
    /// `slot`, which is outside the full mask, in a node with `mask`: two
    /// words for each earlier such leaf, one more outside the default mask
    /// and two more for each mixed leaf. A brick without a full mask (most)
    /// has no default partial leaves, so its stored leaves are records and
    /// mixed leaves: one rank, and one more with a mixed mask, as in
    /// `step`.
    #[inline]
    pub const fn record_offset(self, mask: u64, slot: u32) -> usize {
        if self.full == self.defaults {
            let records = RECORD_WORDS * rank(mask & !self.defaults, slot);
            return if self.pairs == 0 {
                BRICK_HEADER_WORDS + records
            } else {
                BRICK_HEADER_WORDS + 2 + records + 2 * rank(self.pairs, slot)
            };
        }
        self.len()
            + 2 * rank(mask & !self.full, slot)
            + rank(mask & !self.full & !self.defaults, slot)
            + 2 * rank(self.pairs, slot)
    }

    /// Leaf `slot` (set in `mask`) of a node with `mask`, read from `block`,
    /// the node's whole block.
    #[inline]
    pub fn leaf(self, block: &[u32], mask: u64, slot: u32) -> Record {
        let b = bit(slot);
        if self.default_full() & b != 0 {
            return Record::filled(self.default);
        }
        if self.full & b != 0 {
            return Record::filled(self.half(block, slot));
        }
        let at = self.record_offset(mask, slot);
        if self.defaults & b != 0 {
            Record::single(
                u64::from(block[at]) | u64::from(block[at + 1]) << 32,
                self.default,
            )
        } else if self.pairs & b != 0 {
            Record::read_pair(&block[at..at + PAIR_WORDS])
        } else {
            Record::read(&block[at..at + RECORD_WORDS])
        }
    }
}

/// Node indices are below this, so they never collide with uniform words.
pub(crate) const MAX_NODES: u32 = UNIFORM_BIT;

/// The longest node block: a level-1 node with the longest header and 64
/// two-material leaves (more than any brick needs: a pair mask leaves no
/// room for a full mask's leaves).
pub(crate) const MAX_BLOCK_LEN: usize = BRICK_HEADER_MAX_WORDS + PAIR_WORDS * TREE_FANOUT as usize;

// The longest block fits the length bits of a level-1 `info`.
const _: () = assert!(MAX_BLOCK_LEN <= BRICK_LEN_MASK as usize);

impl Node {
    /// Words in the block of this node at `level`: one per child at level 2
    /// and up; the length `info` holds at level 1.
    pub(crate) const fn block_len(self, level: u32) -> usize {
        if level == 1 {
            (self.info & BRICK_LEN_MASK) as usize
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

/// The levels a full path key covers: every coordinate of the world tree.
const KEY_LEVELS: u32 = crate::consts::TREE_DEPTH;

/// The bit shifts of the four stages that spread a coordinate's two-bit
/// digits (one per level) to every sixth bit; digit `l` moves by `4 l` bits,
/// the stage of shift `s` moving the digits whose `l` has bit `s / 4`.
const SPREAD_SHIFTS: [u32; 4] = [32, 16, 8, 4];

/// Where the digits sit after each spreading stage.
const SPREAD_MASKS: [u64; 4] = spread_masks();

const fn spread_masks() -> [u64; 4] {
    let mut masks = [0; 4];
    let mut stage = 0;
    while stage < 4 {
        let mut level = 0;
        while level < KEY_LEVELS {
            // The digit's position once the stages up to this one moved it.
            let mut pos = 2 * level;
            let mut k = 0;
            while k <= stage {
                if level & (8 >> k) != 0 {
                    pos += SPREAD_SHIFTS[k];
                }
                k += 1;
            }
            masks[stage] |= 3 << pos;
            level += 1;
        }
        stage += 1;
    }
    masks
}

/// The two-bit digits of `v` (a coordinate below `4^KEY_LEVELS`) moved to
/// every sixth bit: digit `l` to bit `6 l`.
#[inline(always)]
const fn spread(v: u32) -> u64 {
    let mut x = v as u64 & ((1 << (2 * KEY_LEVELS)) - 1);
    x = (x | x << SPREAD_SHIFTS[0]) & SPREAD_MASKS[0];
    x = (x | x << SPREAD_SHIFTS[1]) & SPREAD_MASKS[1];
    x = (x | x << SPREAD_SHIFTS[2]) & SPREAD_MASKS[2];
    (x | x << SPREAD_SHIFTS[3]) & SPREAD_MASKS[3]
}

/// The inverse of [`spread`] for a key whose digits sit at every sixth bit.
#[inline(always)]
const fn compact(key: u64) -> u32 {
    let mut x = key & SPREAD_MASKS[3];
    x = (x | x >> SPREAD_SHIFTS[3]) & SPREAD_MASKS[2];
    x = (x | x >> SPREAD_SHIFTS[2]) & SPREAD_MASKS[1];
    x = (x | x >> SPREAD_SHIFTS[1]) & SPREAD_MASKS[0];
    ((x | x >> SPREAD_SHIFTS[0]) & ((1 << (2 * KEY_LEVELS)) - 1)) as u32
}

/// [`path_key`] for the whole world tree (`TREE_DEPTH` levels), by bit
/// spreading instead of a loop over the levels.
#[inline(always)]
pub(crate) const fn world_key(local: [u32; 3]) -> u64 {
    spread(local[0]) | spread(local[1]) << 2 | spread(local[2]) << 4
}

/// The world-local coordinate whose [`world_key`] is `key`.
#[inline(always)]
pub(crate) const fn world_key_local(key: u64) -> [u32; 3] {
    [compact(key), compact(key >> 2), compact(key >> 4)]
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
    /// The first `len` words of `words`, taken as they are: no second array
    /// to clear and copy. Panics if `len` is 0 or above `N`.
    pub fn from_array(mask: u64, info: u32, words: [u32; N], len: usize) -> Self {
        assert!(
            (1..=N).contains(&len),
            "a node block holds 1 to {N} words, not {len}"
        );
        Self {
            mask,
            info,
            words,
            len,
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
            info: 9 | BRICK_FULL_MASK,
        };
        assert_eq!(node.block_len(2), 3);
        assert_eq!(node.block_len(9), 3);
        assert_eq!(node.block_len(1), 9, "the length in info, not the flag");
    }

    #[test]
    fn brick_headers_round_trip_and_skip_default_leaves() {
        let h = BrickHeader::short(bit(2) | bit(63), MaterialId(7));
        assert_eq!(h.len(), 3);
        assert_eq!(h.info(9), 9);
        assert_eq!(BrickHeader::read(&h.words(), h.info(9)), h);
        let mask = bit(0) | bit(2) | bit(5) | bit(63);
        assert_eq!(h.record_offset(mask, 0), 3);
        assert_eq!(h.record_offset(mask, 5), 6, "slot 2 has no record");
        assert_eq!(h.leaf(&[], mask, 63), Record::filled(MaterialId(7)));
    }

    #[test]
    fn a_full_mask_stores_other_full_leaves_as_halves_and_default_partial_ones_in_two_words() {
        // Slot 2: full of the default; 3: default partial; 5 and 40: full of
        // other materials; 0: a record; 9: three or more materials, mixed.
        let h = BrickHeader {
            defaults: bit(2) | bit(3),
            default: MaterialId(7),
            full: bit(2) | bit(5) | bit(40),
            pairs: bit(9),
        };
        assert_eq!(h.len(), 7);
        assert_eq!(h.info(18), 18 | BRICK_FULL_MASK | BRICK_PAIR_MASK);
        assert_eq!(BrickHeader::read(&h.words(), h.info(18)), h);
        assert_eq!((h.half_words(), h.leaves_start()), (1, 7));
        let mask = bit(0) | bit(2) | bit(3) | bit(5) | bit(9) | bit(40);
        assert_eq!([0, 3, 9].map(|s| h.record_offset(mask, s)), [7, 10, 12]);
        // The halves end the block.
        let block = [
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            1,
            0,
            UNIFORM_BIT | 3,
            0xF0,
            1,
            6,
            0,
            1234,
            0,
            0,
            4 | 9 << 16,
        ];
        assert_eq!(h.leaf(&block, mask, 2), Record::filled(MaterialId(7)));
        assert_eq!(
            h.leaf(&block, mask, 3),
            Record::single(0xF0 | 1 << 32, MaterialId(7))
        );
        assert_eq!(h.leaf(&block, mask, 5), Record::filled(MaterialId(4)));
        assert_eq!(h.leaf(&block, mask, 40), Record::filled(MaterialId(9)));
        assert_eq!(h.leaf(&block, mask, 0), Record::single(1, MaterialId(3)));
        assert_eq!(h.leaf(&block, mask, 9), Record::palette(6, 1234));
    }

    #[test]
    fn two_material_leaves_are_five_words_after_a_pair_mask() {
        // Slot 1: full of the default; 4: a record; 6 and 9: pairs; 7:
        // full of another material.
        let h = BrickHeader {
            defaults: bit(1),
            default: MaterialId(2),
            full: bit(1) | bit(7),
            pairs: bit(6) | bit(9),
        };
        assert_eq!(h.len(), 7);
        assert_eq!(h.info(30), 30 | BRICK_FULL_MASK | BRICK_PAIR_MASK);
        assert_eq!(BrickHeader::read(&h.words(), h.info(30)), h);
        let mask = bit(1) | bit(4) | bit(6) | bit(7) | bit(9);
        assert_eq!([4, 6, 9].map(|s| h.record_offset(mask, s)), [7, 10, 15]);
        let mut block = [0u32; 21];
        block[20] = 5;
        let pair = Record::pair(0xFF00, 2 | 3 << 16, 0b1010);
        block[15..20].copy_from_slice(&pair.pair_words());
        assert_eq!(h.leaf(&block, mask, 9), pair);
        assert_eq!(h.leaf(&block, mask, 7), Record::filled(MaterialId(5)));
        // Without the full mask, the pair mask follows the default.
        let h = BrickHeader { full: bit(1), ..h };
        assert_eq!((h.len(), h.words()[3]), (5, bit(6) as u32 | bit(9) as u32));
        assert_eq!(BrickHeader::read(&h.words(), h.info(9)), h);
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
    fn world_keys_are_path_keys_and_invert() {
        let mut state = 0x9E37_79B9u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state & ((1 << 20) - 1)
        };
        let edges = [0, 1, 3, 4, 63, 64, (1 << 20) - 1, 1 << 19, 0xAAAAA, 0x55555];
        let mut cases: Vec<[u32; 3]> = Vec::new();
        for &a in &edges {
            for &b in &edges {
                cases.push([a, b, edges[(a ^ b) as usize % edges.len()]]);
            }
        }
        cases.extend((0..100_000).map(|_| [next(), next(), next()]));
        for local in cases {
            let key = world_key(local);
            assert_eq!(key, path_key(local, KEY_LEVELS), "{local:?}");
            assert_eq!(world_key_local(key), local);
        }
    }

    #[test]
    #[should_panic(expected = "1 to 64 words")]
    fn packed_nodes_are_never_empty() {
        PackedNode::<64>::from_array(0, 0, [0; 64], 0);
    }
}
