//! The 16-byte node and the child-word encoding (decision 0004).
//!
//! This layout is a format: serialization and the GPU mirror read it bit for
//! bit, so every fact here is asserted at compile time or covered by tests.
//!
//! - A node is `mask: u64, base: u32, info: u32`. Its level, known during
//!   traversal, decides whether it is a branch (level 1 and up) or a leaf
//!   (level 0).
//! - A branch's non-air children are packed at `words[base..]` in slot order,
//!   one word each, found by popcount ([`rank`]). A word with [`UNIFORM_BIT`]
//!   set is a uniform child with the material id in its low 16 bits; any other
//!   word is a node index. Branch `info` is zero.
//! - A leaf's palette and packed indices are described in [`leaf`].

pub(crate) mod leaf;

use bytemuck::{Pod, Zeroable};

use crate::consts::TREE_FANOUT;
use crate::material::MaterialId;

/// One tree node, branch or leaf. `#[repr(C)]` with no padding, so the
/// 16-byte layout is fixed and node pages cast to bytes for the GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Node {
    /// Bit `i` is set when child (branch) or voxel (leaf) `i` is not air.
    pub mask: u64,
    /// First word of the node's block in the word arena.
    pub base: u32,
    /// Leaf: palette length (low byte) and index bit width (second byte).
    /// Branch: zero.
    pub info: u32,
}

const _: () = assert!(size_of::<Node>() == 16);
const _: () = assert!(align_of::<Node>() == 8);
const _: () = assert!(size_of::<Node>() == size_of::<u64>() + 2 * size_of::<u32>());
const _: () = assert!(TREE_FANOUT == u64::BITS);

impl Node {
    /// Words in this node's block: one per child for a branch, palette plus
    /// indices for a leaf. Always between 1 and 64 for a live node.
    pub(crate) fn block_len(self, kind: NodeKind) -> usize {
        match kind {
            NodeKind::Branch => self.mask.count_ones() as usize,
            NodeKind::Leaf => leaf::block_len(self.mask, self.info),
        }
    }
}

/// How a node is read, decided by its level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Branch,
    Leaf,
}

impl NodeKind {
    pub const fn at_level(level: u32) -> Self {
        if level == 0 { Self::Leaf } else { Self::Branch }
    }
}

/// Set in a child word that holds a uniform material instead of a node index.
pub const UNIFORM_BIT: u32 = 1 << 31;

/// Node indices are below this, so they never collide with uniform words.
pub(crate) const MAX_NODES: u32 = UNIFORM_BIT;

/// One child as the tree stores it. Air children have no word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Child {
    /// Every voxel is air: a clear mask bit.
    #[default]
    Air,
    /// Every voxel is this solid material (never air).
    Uniform(MaterialId),
    /// A node index; the node's level is one below its parent's.
    Node(u32),
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
        if word & UNIFORM_BIT != 0 {
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
        }
    }
}

/// The mask bit of child slot `slot` (0 to 63).
pub(crate) const fn bit(slot: u32) -> u64 {
    1 << slot
}

/// The position of slot `slot` among the set bits of `mask`: the number of
/// set bits below it. This is the index of the slot's word in a packed block.
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

/// A node's content ready to store: its mask, info and block of words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PackedNode {
    pub mask: u64,
    pub info: u32,
    words: [u32; TREE_FANOUT as usize],
    len: usize,
}

impl PackedNode {
    /// Panics if `words` is empty or longer than 64.
    pub fn new(mask: u64, info: u32, words: &[u32]) -> Self {
        assert!(
            (1..=TREE_FANOUT as usize).contains(&words.len()),
            "a node block holds 1 to 64 words, not {}",
            words.len()
        );
        let mut block = [0; TREE_FANOUT as usize];
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
pub(crate) enum Packed {
    /// Every voxel is this material (air included): no node is needed.
    Filled(MaterialId),
    /// Mixed content that needs a node.
    Node(PackedNode),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_words_round_trip() {
        for child in [
            Child::Uniform(MaterialId(1)),
            Child::Uniform(MaterialId(u16::MAX)),
            Child::Node(0),
            Child::Node(MAX_NODES - 1),
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
    fn block_len_follows_the_kind() {
        let branch = Node {
            mask: bit(1) | bit(9) | bit(40),
            base: 0,
            info: 0,
        };
        assert_eq!(branch.block_len(NodeKind::Branch), 3);
        assert_eq!(NodeKind::at_level(0), NodeKind::Leaf);
        assert_eq!(NodeKind::at_level(1), NodeKind::Branch);
    }

    #[test]
    #[should_panic(expected = "1 to 64 words")]
    fn packed_nodes_are_never_empty() {
        PackedNode::new(0, 0, &[]);
    }
}
