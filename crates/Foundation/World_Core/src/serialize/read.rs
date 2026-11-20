//! Reading the body. One walk does both jobs: with [`Validate`] it checks
//! every byte against the format and builds nothing; with [`TreeBuild`] it
//! writes the tree bottom-up, children before parents, each leaf and node
//! packed once at its final size (decision 0006), from a body that has
//! already passed.

use super::{
    DecodeError, HEADER_LEN, TAG_AIR, TAG_BRANCH, TAG_LEAF, TAG_NOT_RESIDENT, TAG_UNIFORM,
    index_bits,
};
use crate::consts::{TREE_DEPTH, TREE_FANOUT};
use crate::material::MaterialId;
use crate::tree::Builder;
use crate::tree::node::leaf::{Record, Voxels};
use crate::tree::node::{CHUNK_LEVEL, Child};

/// The children of one branch, in slot order.
type Slots<T> = [T; TREE_FANOUT as usize];

/// What the walk produces from each child it reads.
pub(super) trait Build {
    /// A child of a level-1 branch: one 4x4x4 leaf.
    type Leaf: Copy;
    /// A child at level 1 or above, or the root.
    type Child: Copy;
    const AIR_LEAF: Self::Leaf;
    const AIR_CHILD: Self::Child;
    /// A leaf whose every voxel is `material`, never air.
    fn uniform_leaf(&mut self, material: MaterialId) -> Self::Leaf;
    /// A leaf of two or more materials, or of one with some air.
    fn mixed_leaf(&mut self, voxels: &Voxels) -> Self::Leaf;
    /// A child whose every voxel is `material`, never air.
    fn uniform(&mut self, material: MaterialId) -> Self::Child;
    /// A child at the chunk level or above whose content is not loaded.
    fn not_resident(&mut self) -> Self::Child;
    /// Called before the children of a branch at the chunk level are read:
    /// they belong to a new chunk.
    fn begin_chunk(&mut self);
    /// A level-1 branch made of these leaves (air where the mask is clear).
    fn brick(&mut self, leaves: &Slots<Self::Leaf>) -> Self::Child;
    /// A branch at `level` (2 and up) made of these children.
    fn branch(&mut self, level: u32, children: &Slots<Self::Child>) -> Self::Child;
}

/// Checks the body and builds nothing.
pub(super) struct Validate;

impl Build for Validate {
    type Leaf = ();
    type Child = ();
    const AIR_LEAF: () = ();
    const AIR_CHILD: () = ();
    fn uniform_leaf(&mut self, _: MaterialId) {}
    fn mixed_leaf(&mut self, _: &Voxels) {}
    fn uniform(&mut self, _: MaterialId) {}
    fn not_resident(&mut self) {}
    fn begin_chunk(&mut self) {}
    fn brick(&mut self, _: &Slots<()>) {}
    fn branch(&mut self, _: u32, _: &Slots<()>) {}
}

/// Writes the tree. Used only on a body [`Validate`] accepted, so every child
/// is canonical and nothing collapses.
pub(super) struct TreeBuild<'a, 'b> {
    pub builder: &'a mut Builder<'b>,
}

impl Build for TreeBuild<'_, '_> {
    type Leaf = Record;
    type Child = Child;
    const AIR_LEAF: Record = Record::AIR;
    const AIR_CHILD: Child = Child::Air;

    fn uniform_leaf(&mut self, material: MaterialId) -> Record {
        Record::filled(material)
    }

    fn mixed_leaf(&mut self, voxels: &Voxels) -> Record {
        self.builder.leaf(voxels)
    }

    fn uniform(&mut self, material: MaterialId) -> Child {
        Child::filled(material)
    }

    fn not_resident(&mut self) -> Child {
        Child::NotResident
    }

    fn begin_chunk(&mut self) {
        self.builder.begin_chunk();
    }

    fn brick(&mut self, leaves: &Slots<Record>) -> Child {
        self.builder.brick(leaves)
    }

    fn branch(&mut self, level: u32, children: &Slots<Child>) -> Child {
        let child = self.builder.branch(level, children);
        if level == CHUNK_LEVEL {
            self.builder.end_chunk(child)
        } else {
            child
        }
    }
}

/// Walks a whole body: the root child, then nothing. Returns the root.
pub(super) fn body<B: Build>(bytes: &[u8], build: &mut B) -> Result<B::Child, DecodeError> {
    let mut r = Reader { bytes, pos: 0 };
    let root = match r.u8()? {
        TAG_AIR => B::AIR_CHILD,
        tag => r.node_child(tag, 0, TREE_DEPTH - 1, build)?.1,
    };
    if r.pos != bytes.len() {
        return Err(malformed(r.pos, "the body continues after the root"));
    }
    Ok(root)
}

/// What a child turned out to be, for its parent's canonical check.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Uniform(u16),
    NotResident,
    Node,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

/// An error at body offset `at`, reported as an offset into the whole input.
fn malformed(at: usize, reason: &'static str) -> DecodeError {
    DecodeError::Malformed {
        offset: HEADER_LEN + at,
        reason,
    }
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let bytes = self.bytes;
        let Some(slice) = bytes.get(self.pos..self.pos + n) else {
            return Err(malformed(self.bytes.len(), "the body ends inside a child"));
        };
        self.pos += n;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let slice = self.take(N)?;
        Ok(std::array::from_fn(|i| slice[i]))
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        self.array().map(u8::from_le_bytes)
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        self.array().map(u16::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        self.array().map(u64::from_le_bytes)
    }

    /// A uniform child's material, refusing air; its tag is at `at`.
    fn uniform_id(&mut self, at: usize) -> Result<u16, DecodeError> {
        let id = self.u16()?;
        if id == 0 {
            return Err(malformed(at, "a uniform child of air"));
        }
        Ok(id)
    }

    /// The child at `level` (1 and up) whose tag, at body offset `at`, was
    /// just read.
    fn node_child<B: Build>(
        &mut self,
        tag: u8,
        at: usize,
        level: u32,
        build: &mut B,
    ) -> Result<(Kind, B::Child), DecodeError> {
        match tag {
            TAG_UNIFORM => {
                let id = self.uniform_id(at)?;
                Ok((Kind::Uniform(id), build.uniform(MaterialId(id))))
            }
            TAG_BRANCH => {
                if level == CHUNK_LEVEL {
                    build.begin_chunk();
                }
                self.branch(at, level, build)
            }
            TAG_NOT_RESIDENT if level < CHUNK_LEVEL => {
                Err(malformed(at, "a not-resident child below the chunk level"))
            }
            TAG_NOT_RESIDENT => Ok((Kind::NotResident, build.not_resident())),
            TAG_LEAF => Err(malformed(at, "a leaf above level 0")),
            TAG_AIR => Err(malformed(at, "an air child inside a branch")),
            _ => Err(malformed(at, "an unknown child tag")),
        }
    }

    /// A child of a level-1 branch whose tag, at body offset `at`, was just
    /// read.
    fn leaf_child<B: Build>(
        &mut self,
        tag: u8,
        at: usize,
        build: &mut B,
    ) -> Result<(Kind, B::Leaf), DecodeError> {
        match tag {
            TAG_UNIFORM => {
                let id = self.uniform_id(at)?;
                Ok((Kind::Uniform(id), build.uniform_leaf(MaterialId(id))))
            }
            TAG_BRANCH => Err(malformed(at, "a branch at level 0")),
            TAG_NOT_RESIDENT => Err(malformed(at, "a not-resident child below the chunk level")),
            TAG_LEAF => self.leaf(at, build),
            TAG_AIR => Err(malformed(at, "an air child inside a branch")),
            _ => Err(malformed(at, "an unknown child tag")),
        }
    }

    fn branch<B: Build>(
        &mut self,
        at: usize,
        level: u32,
        build: &mut B,
    ) -> Result<(Kind, B::Child), DecodeError> {
        let mask = self.u64()?;
        if mask == 0 {
            return Err(malformed(at, "a branch with no children"));
        }
        let mut leaves = [B::AIR_LEAF; TREE_FANOUT as usize];
        let mut children = [B::AIR_CHILD; TREE_FANOUT as usize];
        let mut same = None;
        let mut mixed = false;
        for slot in set_slots(mask) {
            let child_at = self.pos;
            let tag = self.u8()?;
            let kind = if level == 1 {
                let (kind, leaf) = self.leaf_child(tag, child_at, build)?;
                leaves[slot as usize] = leaf;
                kind
            } else {
                let (kind, child) = self.node_child(tag, child_at, level - 1, build)?;
                children[slot as usize] = child;
                kind
            };
            match kind {
                Kind::Node => mixed = true,
                filled if same.is_none_or(|s| s == filled) => same = Some(filled),
                _ => mixed = true,
            }
        }
        if mask == u64::MAX && !mixed {
            return Err(match same {
                Some(Kind::NotResident) => {
                    malformed(at, "a branch whose children are all not resident")
                }
                _ => malformed(at, "a branch whose voxels are all one material"),
            });
        }
        let built = if level == 1 {
            build.brick(&leaves)
        } else {
            build.branch(level, &children)
        };
        Ok((Kind::Node, built))
    }

    fn leaf<B: Build>(&mut self, at: usize, build: &mut B) -> Result<(Kind, B::Leaf), DecodeError> {
        let mask = self.u64()?;
        if mask == 0 {
            return Err(malformed(at, "a leaf with no solid voxels"));
        }
        let len_at = self.pos;
        let len = usize::from(self.u8()?);
        if !(1..=64).contains(&len) {
            return Err(malformed(len_at, "a palette length outside 1 to 64"));
        }
        let mut palette = [0u16; 64];
        for i in 0..len {
            let id_at = self.pos;
            let id = self.u16()?;
            if id == 0 {
                return Err(malformed(id_at, "air in a palette"));
            }
            if i > 0 && id <= palette[i - 1] {
                return Err(malformed(id_at, "a palette that is not strictly ascending"));
            }
            palette[i] = id;
        }

        let bits = index_bits(len);
        let solids = mask.count_ones() as usize;
        let packed_at = self.pos;
        let packed = self.take((solids * bits).div_ceil(8))?;
        let index_mask = ((1u16 << bits) - 1) as u8;
        let mut used = 0u64;
        let mut voxels: Voxels = [MaterialId::AIR; TREE_FANOUT as usize];
        for (k, slot) in set_slots(mask).enumerate() {
            // The width divides 8, so an index never straddles a byte. With
            // zero-width indices every voxel is palette entry 0.
            let bit = k * bits;
            let index = if bits == 0 {
                0
            } else {
                usize::from(packed[bit / 8] >> (bit % 8) & index_mask)
            };
            if index >= len {
                return Err(malformed(
                    packed_at + bit / 8,
                    "a palette index past the palette",
                ));
            }
            used |= 1 << index;
            voxels[slot as usize] = MaterialId(palette[index]);
        }
        let tail = solids * bits % 8;
        if let Some(&last) = packed.last()
            && tail != 0
            && last >> tail != 0
        {
            return Err(malformed(
                packed_at + packed.len() - 1,
                "set bits after the last index",
            ));
        }
        if used.count_ones() as usize != len {
            return Err(malformed(len_at, "a palette entry that no voxel uses"));
        }
        if mask == u64::MAX && len == 1 {
            return Err(malformed(at, "a leaf whose voxels are all one material"));
        }
        Ok((Kind::Node, build.mixed_leaf(&voxels)))
    }
}

/// The set bits of a mask, ascending.
fn set_slots(mask: u64) -> impl Iterator<Item = u32> {
    let mut rest = mask;
    std::iter::from_fn(move || {
        (rest != 0).then(|| {
            let slot = rest.trailing_zeros();
            rest &= rest - 1;
            slot
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::WORLD_MIN_VOXEL;
    use crate::coords::VoxelPos;
    use crate::tree::VoxelTree;

    fn check(bytes: &[u8]) -> Result<(), DecodeError> {
        body(bytes, &mut Validate)
    }

    /// Builds a tree from a body that passed [`check`].
    fn build(bytes: &[u8]) -> VoxelTree {
        check(bytes).expect("a valid body");
        VoxelTree::build_with(|builder| body(bytes, &mut TreeBuild { builder })).unwrap()
    }

    /// The reason a body is rejected for.
    fn reason(bytes: &[u8]) -> &'static str {
        match check(bytes) {
            Err(DecodeError::Malformed { reason, .. }) => reason,
            other => panic!("expected a malformed body, got {other:?}"),
        }
    }

    /// Branches from the root down to `level`, each holding only `slot`, with
    /// `tail` as the one child that stands at `level`.
    fn below(level: u32, slot: u32, tail: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for _ in level..TREE_DEPTH - 1 {
            bytes.push(TAG_BRANCH);
            bytes.extend_from_slice(&(1u64 << slot).to_le_bytes());
        }
        bytes.extend_from_slice(tail);
        bytes
    }

    /// A leaf: mask, palette and packed index bytes.
    fn leaf(mask: u64, palette: &[u16], packed: &[u8]) -> Vec<u8> {
        let mut bytes = vec![TAG_LEAF];
        bytes.extend_from_slice(&mask.to_le_bytes());
        bytes.push(palette.len() as u8);
        for id in palette {
            bytes.extend_from_slice(&id.to_le_bytes());
        }
        bytes.extend_from_slice(packed);
        bytes
    }

    fn uniform(id: u16) -> Vec<u8> {
        let mut bytes = vec![TAG_UNIFORM];
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes
    }

    #[test]
    fn valid_bodies_pass() {
        assert_eq!(check(&[TAG_AIR]), Ok(()));
        assert_eq!(check(&uniform(3)), Ok(()));
        assert_eq!(check(&below(0, 0, &uniform(3))), Ok(()));
        // Two voxels of two materials: indices 0 then 1, one bit each.
        assert_eq!(check(&below(0, 0, &leaf(0b11, &[4, 9], &[0b10]))), Ok(()));
        // A full leaf of 64 distinct materials, eight-bit indices.
        let palette: Vec<u16> = (1..=64).collect();
        let packed: Vec<u8> = (0..64).collect();
        assert_eq!(
            check(&below(0, 0, &leaf(u64::MAX, &palette, &packed))),
            Ok(())
        );
    }

    #[test]
    fn voxels_land_where_their_slot_path_says() {
        // Below the root, every branch down to level 2 holds only slot 63.
        // The level-1 branch holds a leaf in slot 0 and a uniform in slot 63.
        let mut tail = vec![TAG_BRANCH];
        tail.extend_from_slice(&(1u64 << 63 | 1).to_le_bytes());
        tail.extend_from_slice(&leaf(1 << 21, &[6], &[]));
        tail.extend_from_slice(&uniform(7));
        let tree = build(&below(1, 63, &tail));

        // Slot 63 of a level-L node is offset 3 * 4^L on every axis.
        let corner: i32 = (2..TREE_DEPTH).map(|l| 3 * 4i32.pow(l)).sum();
        let base = WORLD_MIN_VOXEL + corner;
        let at = |d: i32| VoxelPos::new(base + d, base + d, base + d);
        // Voxel slot 21 of the leaf is (1, 1, 1); its neighbours are air.
        assert_eq!(tree.get(at(1)), MaterialId(6));
        assert_eq!(tree.get(at(0)), MaterialId::AIR);
        assert_eq!(tree.get(at(2)), MaterialId::AIR);
        // The uniform leaf covers (12, 12, 12) up to (16, 16, 16).
        assert_eq!(tree.get(at(12)), MaterialId(7));
        assert_eq!(tree.get(at(15)), MaterialId(7));
        assert_eq!(tree.get(at(16)), MaterialId::AIR);
        let s = tree.stats();
        assert_eq!((s.branches, s.leaves), (TREE_DEPTH as usize - 1, 1));
    }

    #[test]
    fn a_uniform_root_fills_the_world() {
        let tree = build(&uniform(2));
        assert_eq!(tree.get(VoxelPos::new(0, 0, 0)), MaterialId(2));
        assert_eq!(
            tree.get(VoxelPos::new(
                WORLD_MIN_VOXEL,
                WORLD_MIN_VOXEL,
                WORLD_MIN_VOXEL
            )),
            MaterialId(2)
        );
        assert_eq!(tree.stats().branches, 0);
    }

    #[test]
    fn mixed_leaves_keep_every_voxel() {
        let palette: Vec<u16> = (1..=64).collect();
        let packed: Vec<u8> = (0..64).collect();
        let tree = build(&below(0, 0, &leaf(u64::MAX, &palette, &packed)));
        let o = WORLD_MIN_VOXEL;
        for slot in 0..64 {
            let pos = VoxelPos::new(o + slot % 4, o + slot / 4 % 4, o + slot / 16);
            assert_eq!(tree.get(pos), MaterialId(slot as u16 + 1), "slot {slot}");
        }
        assert_eq!(tree.stats().leaves, 1);
    }

    #[test]
    fn structure_errors() {
        assert_eq!(reason(&[]), "the body ends inside a child");
        assert_eq!(reason(&[9]), "an unknown child tag");
        assert_eq!(reason(&[TAG_AIR, 0]), "the body continues after the root");
        assert_eq!(reason(&uniform(0)), "a uniform child of air");
        assert_eq!(
            reason(&below(0, 0, &[TAG_AIR])),
            "an air child inside a branch"
        );
        assert_eq!(
            reason(&below(0, 0, &[TAG_BRANCH, 1, 0, 0, 0, 0, 0, 0, 0])),
            "a branch at level 0"
        );
        assert_eq!(
            reason(&below(1, 0, &leaf(1, &[2], &[]))),
            "a leaf above level 0"
        );
        assert_eq!(
            reason(&[TAG_BRANCH, 0, 0, 0, 0, 0, 0, 0, 0]),
            "a branch with no children"
        );
        assert_eq!(
            reason(&below(1, 0, &[TAG_BRANCH, 1, 2])),
            "the body ends inside a child"
        );
    }

    #[test]
    fn a_branch_of_one_material_is_not_canonical() {
        let mut all = vec![TAG_BRANCH];
        all.extend_from_slice(&u64::MAX.to_le_bytes());
        for _ in 0..64 {
            all.extend_from_slice(&uniform(5));
        }
        assert_eq!(
            reason(&below(1, 0, &all)),
            "a branch whose voxels are all one material"
        );
        // One child of another material makes it a real branch.
        let last = all.len() - 2;
        all[last] = 6;
        assert_eq!(check(&below(1, 0, &all)), Ok(()));
    }

    #[test]
    fn leaf_errors() {
        let at0 = |l: Vec<u8>| reason(&below(0, 0, &l));
        assert_eq!(at0(leaf(0, &[1], &[])), "a leaf with no solid voxels");
        assert_eq!(at0(leaf(1, &[], &[])), "a palette length outside 1 to 64");
        assert_eq!(at0(leaf(1, &[0], &[])), "air in a palette");
        assert_eq!(
            at0(leaf(0b11, &[4, 4], &[0b10])),
            "a palette that is not strictly ascending"
        );
        assert_eq!(
            at0(leaf(0b11, &[5, 4], &[0b10])),
            "a palette that is not strictly ascending"
        );
        // Three entries take two bits each; the third index, 3, is past them.
        assert_eq!(
            at0(leaf(0b111, &[1, 2, 3], &[0b11_01_00])),
            "a palette index past the palette"
        );
        assert_eq!(
            at0(leaf(0b11, &[4, 9], &[0b110])),
            "set bits after the last index"
        );
        assert_eq!(
            at0(leaf(0b11, &[4, 9], &[0b00])),
            "a palette entry that no voxel uses"
        );
        assert_eq!(
            at0(leaf(0b1, &[4, 9], &[0b0])),
            "a palette entry that no voxel uses"
        );
        assert_eq!(
            at0(leaf(u64::MAX, &[4], &[])),
            "a leaf whose voxels are all one material"
        );
        assert_eq!(
            at0(leaf(0b11, &[4, 9], &[])),
            "the body ends inside a child"
        );
        let mut long = leaf(1, &[1], &[]);
        long[9] = 65;
        assert_eq!(at0(long), "a palette length outside 1 to 64");
    }

    #[test]
    fn offsets_point_into_the_whole_input() {
        let bytes = below(0, 0, &uniform(0));
        let tag_at = bytes.len() - 3;
        assert_eq!(
            check(&bytes),
            Err(DecodeError::Malformed {
                offset: HEADER_LEN + tag_at,
                reason: "a uniform child of air"
            })
        );
    }
}
