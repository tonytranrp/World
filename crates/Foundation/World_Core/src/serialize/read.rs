//! Reading the body. One walk does both jobs: with [`Validate`] it checks
//! every byte against the format and builds nothing; with a [`VoxelTree`] it
//! builds the tree from a body that has already passed.

use super::{DecodeError, HEADER_LEN, TAG_AIR, TAG_BRANCH, TAG_LEAF, TAG_UNIFORM, index_bits};
use crate::consts::TREE_DEPTH;
use crate::coords::{VoxelBox, VoxelPos, path};
use crate::material::MaterialId;
use crate::tree::VoxelTree;

/// What the walk produces: whole regions and single voxels of the content.
pub(super) trait Sink {
    /// Every voxel of `region` is `material`.
    fn uniform(&mut self, region: VoxelBox, material: MaterialId);
    /// One solid voxel.
    fn voxel(&mut self, pos: VoxelPos, material: MaterialId);
}

/// Checks the body and builds nothing.
pub(super) struct Validate;

impl Sink for Validate {
    fn uniform(&mut self, _: VoxelBox, _: MaterialId) {}
    fn voxel(&mut self, _: VoxelPos, _: MaterialId) {}
}

/// Regions and voxels come from slot paths below the root, so they always lie
/// inside the world and the tree cannot refuse them.
impl Sink for VoxelTree {
    fn uniform(&mut self, region: VoxelBox, material: MaterialId) {
        self.fill_box(region, material)
            .expect("a decoded region lies inside the world");
    }

    fn voxel(&mut self, pos: VoxelPos, material: MaterialId) {
        self.set(pos, material)
            .expect("a decoded voxel lies inside the world");
    }
}

/// Walks a whole body: the root child, then nothing.
pub(super) fn body(bytes: &[u8], sink: &mut impl Sink) -> Result<(), DecodeError> {
    let mut r = Reader { bytes, pos: 0 };
    match r.u8()? {
        TAG_AIR => {}
        tag => {
            r.child(tag, 0, TREE_DEPTH - 1, [0; 3], sink)?;
        }
    }
    if r.pos != bytes.len() {
        return Err(malformed(r.pos, "the body continues after the root"));
    }
    Ok(())
}

/// What a child turned out to be, for its parent's canonical check.
#[derive(Clone, Copy)]
enum Kind {
    Uniform(u16),
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

    /// The child whose tag (at body offset `at`) was just read. `level` is the
    /// level a node in its place would have; `origin` is the world-local
    /// lowest voxel of its region.
    fn child(
        &mut self,
        tag: u8,
        at: usize,
        level: u32,
        origin: [u32; 3],
        sink: &mut impl Sink,
    ) -> Result<Kind, DecodeError> {
        match tag {
            TAG_UNIFORM => {
                let id = self.u16()?;
                if id == 0 {
                    return Err(malformed(at, "a uniform child of air"));
                }
                let span = path::child_span(level + 1);
                let region = VoxelBox::from_min_size(path::from_world_local(origin), [span; 3]);
                sink.uniform(region, MaterialId(id));
                Ok(Kind::Uniform(id))
            }
            TAG_BRANCH if level == 0 => Err(malformed(at, "a branch at level 0")),
            TAG_BRANCH => self.branch(at, level, origin, sink),
            TAG_LEAF if level != 0 => Err(malformed(at, "a leaf above level 0")),
            TAG_LEAF => self.leaf(at, origin, sink),
            TAG_AIR => Err(malformed(at, "an air child inside a branch")),
            _ => Err(malformed(at, "an unknown child tag")),
        }
    }

    fn branch(
        &mut self,
        at: usize,
        level: u32,
        origin: [u32; 3],
        sink: &mut impl Sink,
    ) -> Result<Kind, DecodeError> {
        let mask = self.u64()?;
        if mask == 0 {
            return Err(malformed(at, "a branch with no children"));
        }
        let span = path::child_span(level);
        let mut material = None;
        let mut mixed = false;
        for slot in set_slots(mask) {
            let child_at = self.pos;
            let tag = self.u8()?;
            let offset = path::slot_offset(slot);
            let child_origin = std::array::from_fn(|a| origin[a] + offset[a] * span);
            match self.child(tag, child_at, level - 1, child_origin, sink)? {
                Kind::Uniform(id) if material.is_none_or(|m| m == id) => material = Some(id),
                _ => mixed = true,
            }
        }
        if mask == u64::MAX && !mixed {
            return Err(malformed(at, "a branch whose voxels are all one material"));
        }
        Ok(Kind::Node)
    }

    fn leaf(
        &mut self,
        at: usize,
        origin: [u32; 3],
        sink: &mut impl Sink,
    ) -> Result<Kind, DecodeError> {
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
            let offset = path::slot_offset(slot);
            let local = std::array::from_fn(|a| origin[a] + offset[a]);
            sink.voxel(path::from_world_local(local), MaterialId(palette[index]));
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
        Ok(Kind::Node)
    }
}

/// The set bits of a mask, ascending.
fn set_slots(mask: u64) -> impl Iterator<Item = u32> {
    (0..64).filter(move |s| mask >> s & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::WORLD_MIN_VOXEL;

    /// Records what the walk produces, to check positions without a tree.
    #[derive(Default)]
    struct Record {
        regions: Vec<(VoxelBox, MaterialId)>,
        voxels: Vec<(VoxelPos, MaterialId)>,
    }

    impl Sink for Record {
        fn uniform(&mut self, region: VoxelBox, material: MaterialId) {
            self.regions.push((region, material));
        }

        fn voxel(&mut self, pos: VoxelPos, material: MaterialId) {
            self.voxels.push((pos, material));
        }
    }

    fn check(bytes: &[u8]) -> Result<(), DecodeError> {
        body(bytes, &mut Validate)
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
    fn regions_and_voxels_land_where_their_slot_path_says() {
        // Below the root, every branch down to level 2 holds only slot 63.
        // The level-1 branch holds a leaf in slot 0 and a uniform in slot 63.
        let mut tail = vec![TAG_BRANCH];
        tail.extend_from_slice(&(1u64 << 63 | 1).to_le_bytes());
        tail.extend_from_slice(&leaf(1 << 21, &[6], &[]));
        tail.extend_from_slice(&uniform(7));
        let bytes = below(1, 63, &tail);
        let mut record = Record::default();
        assert_eq!(body(&bytes, &mut record), Ok(()));

        // Slot 63 of a level-L node is offset 3 * 4^L on every axis.
        let corner: i32 = (2..TREE_DEPTH).map(|l| 3 * 4i32.pow(l)).sum();
        let base = WORLD_MIN_VOXEL + corner;
        // Voxel slot 21 is (1, 1, 1).
        assert_eq!(
            record.voxels,
            [(VoxelPos::new(base + 1, base + 1, base + 1), MaterialId(6))]
        );
        // The uniform child covers (12, 12, 12) up to (16, 16, 16).
        assert_eq!(
            record.regions,
            [(
                VoxelBox::from_min_size(VoxelPos::new(base + 12, base + 12, base + 12), [4; 3]),
                MaterialId(7)
            )]
        );
    }

    #[test]
    fn a_uniform_root_covers_the_world() {
        let mut record = Record::default();
        assert_eq!(body(&uniform(2), &mut record), Ok(()));
        assert_eq!(record.regions, [(VoxelBox::WORLD, MaterialId(2))]);
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
