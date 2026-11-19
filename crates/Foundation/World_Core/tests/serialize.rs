//! Tree serialization: lossless round trips, one encoding per content, frozen
//! golden bytes, and hostile input (truncated, flipped, random) that always
//! ends in an error or in a tree that encodes back to exactly the input,
//! never in a panic.

use proptest::collection::vec;
use proptest::prelude::*;
use proptest::sample::Index;
use world_core::consts::TREE_DEPTH;
use world_core::serialize::{DecodeError, FORMAT_VERSION, MAGIC, content_hash, decode, encode};
use world_core::tree::{NodeRef, same_content};
use world_core::{MaterialId, TreeRead, VoxelBox, VoxelPos, VoxelTree};
use world_testkit::dense::{
    Bounds, DenseWorld, EditOp, VoxelModel, assert_tree_agrees, check_canonical,
};
use world_testkit::hash::hash_bytes;
use world_testkit::profile::{proptest_config, scale};
use world_testkit::strategies::edit_ops;

/// Crosses the origin, so trees have branches, leaves and uniform children
/// at many levels.
const B: Bounds = Bounds::new(VoxelPos::new(-21, -10, -17), [40, 24, 36]);

/// Header bytes before the body, and checksum bytes after it.
const HEADER: usize = 15;
const CHECKSUM: usize = 8;

fn build(ops: &[EditOp]) -> (VoxelTree, DenseWorld) {
    let (mut tree, mut dense) = (VoxelTree::new(), DenseWorld::new(B));
    for &op in ops {
        tree.apply_op(op);
        dense.apply_op(op);
    }
    (tree, dense)
}

fn body(bytes: &[u8]) -> &[u8] {
    &bytes[HEADER..bytes.len() - CHECKSUM]
}

/// A header, `body` and its checksum, as `encode` would frame them.
fn seal(body: &[u8]) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes.push(TREE_DEPTH as u8);
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(&hash_bytes(body).to_le_bytes());
    bytes
}

/// Decoding either fails or yields a canonical tree whose encoding is
/// exactly the input: there is one encoding per content.
fn decodes_faithfully_or_fails(bytes: &[u8]) -> Result<(), TestCaseError> {
    if let Ok(tree) = decode(bytes) {
        prop_assert!(check_canonical(&tree).is_ok());
        prop_assert_eq!(encode(&tree), bytes);
    }
    Ok(())
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn round_trips_are_lossless(ops in edit_ops(B, 5, scale().edits_per_case)) {
        let (tree, dense) = build(&ops);
        let bytes = encode(&tree);
        let back = decode(&bytes).unwrap();
        prop_assert!(same_content(&tree, &back));
        prop_assert_eq!(assert_tree_agrees(&back, &dense, B), Ok(()));
        prop_assert!(check_canonical(&back).is_ok());
        prop_assert_eq!(encode(&back), bytes.as_slice());
        prop_assert_eq!(content_hash(&tree), hash_bytes(body(&bytes)));
        prop_assert_eq!(content_hash(&back), content_hash(&tree));
        prop_assert_eq!(encode(&tree.snapshot()), bytes);
    }

    /// The final voxels set one by one in reverse give the same bytes as the
    /// random steps that produced them.
    #[test]
    fn equal_content_gives_identical_bytes(ops in edit_ops(B, 3, scale().edits_per_case)) {
        let (tree, dense) = build(&ops);
        let mut rebuilt = VoxelTree::new();
        let mut solids: Vec<_> = dense.solids().collect();
        solids.reverse();
        for (p, m) in solids {
            rebuilt.set(p, m).unwrap();
        }
        prop_assert_eq!(encode(&rebuilt), encode(&tree));
    }

    #[test]
    fn every_truncation_fails(ops in edit_ops(B, 5, 16)) {
        let bytes = encode(&build(&ops).0);
        for len in 0..bytes.len() {
            prop_assert!(decode(&bytes[..len]).is_err(), "length {}", len);
        }
    }

    /// The header fields and the checksum catch any single changed byte.
    #[test]
    fn any_flipped_byte_is_caught(
        ops in edit_ops(B, 5, 16),
        at in any::<Index>(),
        flip in 1u8..=255,
    ) {
        let mut bytes = encode(&build(&ops).0);
        let i = at.index(bytes.len());
        bytes[i] ^= flip;
        prop_assert!(decode(&bytes).is_err(), "flip {:#x} at byte {}", flip, i);
    }

    /// With the checksum rewritten, a flipped body byte reaches the parser,
    /// which must reject it or read a different, valid tree.
    #[test]
    fn a_resealed_flip_is_rejected_or_read_faithfully(
        ops in edit_ops(B, 5, 16),
        at in any::<Index>(),
        flip in 1u8..=255,
    ) {
        let bytes = encode(&build(&ops).0);
        let mut changed = body(&bytes).to_vec();
        let i = at.index(changed.len());
        changed[i] ^= flip;
        decodes_faithfully_or_fails(&seal(&changed))?;
    }

    #[test]
    fn random_bytes_never_panic(bytes in vec(any::<u8>(), 0..256)) {
        decodes_faithfully_or_fails(&bytes)?;
        let mut framed = MAGIC.to_vec();
        framed.extend_from_slice(&bytes);
        decodes_faithfully_or_fails(&framed)?;
    }

    /// Random bodies in a valid frame exercise the parser itself. Bodies are
    /// drawn from the bytes a body is made of, so they often get deep.
    #[test]
    fn random_bodies_never_panic(
        body in vec(prop_oneof![0u8..=4, Just(0xFF), any::<u8>()], 0..300),
    ) {
        decodes_faithfully_or_fails(&seal(&body))?;
    }
}

#[test]
fn the_empty_tree_encodes_to_these_exact_bytes() {
    #[rustfmt::skip]
    let expected: [u8; 24] = [
        b'W', b'V', b'T', 0,                    // magic
        1, 0,                                   // format version 1
        10,                                     // tree depth
        1, 0, 0, 0, 0, 0, 0, 0,                 // body length 1
        0,                                      // the root is air
        0xdb, 0xec, 0x4e, 0x07, 0xf4, 0xdf, 0x4b, 0xc4, // xxh3-64 of the body
    ];
    let tree = VoxelTree::new();
    assert_eq!(encode(&tree), expected);
    assert_eq!(content_hash(&tree), 0xc44b_dff4_074e_ecdb);
    assert!(matches!(decode(&expected).unwrap().root(), NodeRef::Air));
}

#[test]
fn a_world_of_one_material_encodes_to_these_exact_bytes() {
    #[rustfmt::skip]
    let expected: [u8; 26] = [
        b'W', b'V', b'T', 0, 1, 0, 10,
        3, 0, 0, 0, 0, 0, 0, 0,                 // body length 3
        1, 7, 0,                                // a uniform root of material 7
        0x2a, 0xe5, 0x6d, 0xd7, 0x93, 0x4f, 0x66, 0x36,
    ];
    let mut tree = VoxelTree::new();
    tree.fill_box(VoxelBox::WORLD, MaterialId(7)).unwrap();
    assert_eq!(encode(&tree), expected);
    assert_eq!(content_hash(&tree), 0x3666_4f93_d76d_e52a);
    let back = decode(&expected).unwrap();
    assert!(matches!(back.root(), NodeRef::Uniform(MaterialId(7))));
}

#[test]
fn one_voxel_encodes_as_a_path_of_branches_and_a_leaf() {
    let mut tree = VoxelTree::new();
    tree.set(VoxelPos::ORIGIN, MaterialId(0x0203)).unwrap();
    let bytes = encode(&tree);
    // The origin is slot 42 at the root and slot 0 below it.
    let mut expected = Vec::new();
    for level in (1..TREE_DEPTH).rev() {
        let slot = if level == TREE_DEPTH - 1 { 42 } else { 0 };
        expected.push(2);
        expected.extend_from_slice(&(1u64 << slot).to_le_bytes());
    }
    expected.push(3);
    expected.extend_from_slice(&1u64.to_le_bytes());
    expected.extend_from_slice(&[1, 0x03, 0x02]);
    assert_eq!(body(&bytes), expected.as_slice());
    let back = decode(&bytes).unwrap();
    assert_eq!(TreeRead::get(&back, VoxelPos::ORIGIN), MaterialId(0x0203));
}

#[test]
fn header_errors_are_specific() {
    let mut tree = VoxelTree::new();
    tree.set(VoxelPos::new(3, -4, 5), MaterialId(2)).unwrap();
    let good = encode(&tree);

    let mut magic = good.clone();
    magic[0] = b'X';
    assert_eq!(decode(&magic).err(), Some(DecodeError::NotATree));

    let mut version = good.clone();
    version[4..6].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
    assert_eq!(
        decode(&version).err(),
        Some(DecodeError::UnsupportedVersion {
            found: FORMAT_VERSION + 1
        })
    );

    let mut depth = good.clone();
    depth[6] = TREE_DEPTH as u8 + 1;
    assert_eq!(
        decode(&depth).err(),
        Some(DecodeError::WrongDepth {
            found: TREE_DEPTH as u8 + 1
        })
    );

    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(decode(&trailing).err(), Some(DecodeError::TrailingBytes));

    assert_eq!(
        decode(&good[..good.len() - 1]).err(),
        Some(DecodeError::Truncated)
    );

    let mut corrupt = good;
    corrupt[HEADER] ^= 0x40;
    assert_eq!(decode(&corrupt).err(), Some(DecodeError::ChecksumMismatch));
}

#[test]
fn different_content_hashes_differently() {
    let mut a = VoxelTree::new();
    let mut b = VoxelTree::new();
    a.set(VoxelPos::new(1, 2, 3), MaterialId(4)).unwrap();
    b.set(VoxelPos::new(1, 2, 3), MaterialId(5)).unwrap();
    assert_ne!(content_hash(&a), content_hash(&b));
    b.set(VoxelPos::new(1, 2, 3), MaterialId(4)).unwrap();
    assert_eq!(content_hash(&a), content_hash(&b));
}

#[test]
fn a_fill_and_its_voxels_encode_identically() {
    let region = VoxelBox::from_min_size(VoxelPos::new(-6, -2, 9), [13, 5, 8]);
    let mut by_fill = VoxelTree::new();
    by_fill.fill_box(region, MaterialId(3)).unwrap();
    let mut by_voxel = VoxelTree::new();
    for z in (region.min().z..region.end().z).rev() {
        for x in region.min().x..region.end().x {
            for y in region.min().y..region.end().y {
                by_voxel.set(VoxelPos::new(x, y, z), MaterialId(3)).unwrap();
            }
        }
    }
    assert_eq!(encode(&by_fill), encode(&by_voxel));
}
