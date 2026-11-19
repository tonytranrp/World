//! Canonical form: equal content means equal structure, equal live storage
//! and identical bytes, whatever edits produced it; emptied space frees its
//! nodes; and whole node regions of one material are uniform children.

use proptest::prelude::*;
use world_core::consts::{TREE_DEPTH, WORLD_MIN_VOXEL};
use world_core::coords::path;
use world_core::serialize::{content_hash, encode};
use world_core::tree::{NodeRef, same_content};
use world_core::{MaterialId, TreeRead, VoxelBox, VoxelPos, VoxelTree};
use world_testkit::dense::{Bounds, DenseWorld, EditOp, NodeCounts, VoxelModel, check_canonical};
use world_testkit::profile::{proptest_config, scale};
use world_testkit::strategies::edit_ops;

/// Crosses the origin, so it splits nodes of every level.
const SMALL: Bounds = Bounds::new(VoxelPos::new(-9, -5, -12), [24, 20, 28]);

/// The live counts, after checking that the walk agrees with them.
fn canonical_counts(tree: &VoxelTree) -> Result<NodeCounts, TestCaseError> {
    let counts = check_canonical(tree).map_err(TestCaseError::fail)?;
    prop_assert_eq!(NodeCounts::from(tree.stats()), counts);
    Ok(counts)
}

/// Everything two trees of equal content must share.
fn assert_same_structure(a: &VoxelTree, b: &VoxelTree) -> Result<(), TestCaseError> {
    prop_assert!(same_content(a, b));
    prop_assert_eq!(canonical_counts(a)?, canonical_counts(b)?);
    prop_assert_eq!(encode(a), encode(b));
    prop_assert_eq!(content_hash(a), content_hash(b));
    Ok(())
}

fn voxels(b: Bounds) -> impl Iterator<Item = VoxelPos> {
    let lo = b.min;
    let [sx, sy, sz] = b.size.map(|s| s as i32);
    (0..sz).flat_map(move |z| {
        (0..sy).flat_map(move |y| (0..sx).map(move |x| VoxelPos::new(lo.x + x, lo.y + y, lo.z + z)))
    })
}

fn build(ops: &[EditOp]) -> (VoxelTree, DenseWorld) {
    let (mut tree, mut dense) = (VoxelTree::new(), DenseWorld::new(SMALL));
    for &op in ops {
        tree.apply_op(op);
        dense.apply_op(op);
    }
    (tree, dense)
}

fn assert_empty(tree: &VoxelTree) -> Result<(), TestCaseError> {
    prop_assert!(matches!(tree.root(), NodeRef::Air));
    prop_assert_eq!(canonical_counts(tree)?, NodeCounts::default());
    Ok(())
}

proptest! {
    #![proptest_config(proptest_config())]

    /// The same final voxels three ways: the random steps in order; only the
    /// solid voxels, set one by one in reverse scan order; and the region
    /// filled solid first, then every voxel set to its final value.
    #[test]
    fn equal_content_has_equal_structure_whatever_the_edits(
        ops in edit_ops(SMALL, 3, scale().edits_per_case),
    ) {
        let (by_ops, dense) = build(&ops);

        let mut by_voxels = VoxelTree::new();
        let mut solids: Vec<_> = dense.solids().collect();
        solids.reverse();
        for (p, m) in solids {
            by_voxels.set(p, m).unwrap();
        }

        let mut by_carving = VoxelTree::new();
        by_carving.fill_box(SMALL.as_box(), MaterialId(1)).unwrap();
        for p in voxels(SMALL) {
            by_carving.set(p, dense.get(p)).unwrap();
        }

        assert_same_structure(&by_ops, &by_voxels)?;
        assert_same_structure(&by_ops, &by_carving)?;
    }

    #[test]
    fn clearing_everything_frees_every_node(ops in edit_ops(SMALL, 3, scale().edits_per_case)) {
        let (mut by_box, dense) = build(&ops);
        by_box.fill_box(SMALL.as_box(), MaterialId::AIR).unwrap();
        assert_empty(&by_box)?;

        let (mut by_voxel, _) = build(&ops);
        for (p, _) in dense.solids() {
            by_voxel.set(p, MaterialId::AIR).unwrap();
        }
        assert_empty(&by_voxel)?;
    }
}

/// The child of the level-`level` node on the way to `pos`, after checking
/// that every node above it is a branch with that single child.
fn child_at(tree: &impl TreeRead, pos: VoxelPos, level: u32) -> NodeRef<'_> {
    let local = path::world_local(pos).unwrap();
    let mut node = tree.root();
    for l in (level..TREE_DEPTH).rev() {
        let NodeRef::Branch(b) = node else {
            panic!("expected a branch at level {l}, found {node:?}");
        };
        assert_eq!(b.level(), l);
        let slot = path::child_slot(local, l);
        assert_eq!(b.mask(), 1 << slot, "level {l}");
        node = b.child(slot);
    }
    node
}

/// Corners of `span`-aligned regions on both sides of the origin and at the
/// world's low corner.
fn aligned_corners(span: u32) -> [VoxelPos; 3] {
    let s = span as i32;
    [
        VoxelPos::new(0, 0, 0),
        VoxelPos::new(-s, 2 * s, -3 * s),
        VoxelPos::new(WORLD_MIN_VOXEL, WORLD_MIN_VOXEL, WORLD_MIN_VOXEL + s),
    ]
}

#[test]
fn filling_a_whole_node_region_makes_a_uniform_child() {
    for level in 1..=6 {
        let span = path::child_span(level);
        for corner in aligned_corners(span) {
            let mut tree = VoxelTree::new();
            let region = VoxelBox::from_min_size(corner, [span; 3]);
            assert_eq!(tree.fill_box(region, MaterialId(4)), Ok(true));
            let child = child_at(&tree, corner, level);
            assert!(
                matches!(child, NodeRef::Uniform(MaterialId(4))),
                "level {level} at {corner:?}: {child:?}"
            );
            let path_len = (TREE_DEPTH - level) as usize;
            let expected = NodeCounts {
                branches: path_len,
                leaves: 0,
                words: path_len,
            };
            assert_eq!(check_canonical(&tree), Ok(expected));
            assert_eq!(NodeCounts::from(tree.stats()), expected);
        }
    }
}

#[test]
fn filling_the_whole_world_makes_a_uniform_root() {
    let mut tree = VoxelTree::new();
    tree.fill_box(VoxelBox::WORLD, MaterialId(2)).unwrap();
    assert!(matches!(tree.root(), NodeRef::Uniform(MaterialId(2))));
    assert_eq!(NodeCounts::from(tree.stats()), NodeCounts::default());
}

#[test]
fn a_leaf_filled_voxel_by_voxel_becomes_a_uniform_child() {
    let corner = VoxelPos::new(-4, 8, -12);
    let block = VoxelBox::from_min_size(corner, [4; 3]);
    let mut by_voxel = VoxelTree::new();
    for z in 0..4 {
        for y in 0..4 {
            for x in 0..4 {
                let p = VoxelPos::new(corner.x + x, corner.y + y, corner.z + z);
                by_voxel.set(p, MaterialId(6)).unwrap();
            }
        }
    }
    assert!(matches!(
        child_at(&by_voxel, corner, 1),
        NodeRef::Uniform(MaterialId(6))
    ));
    let mut by_fill = VoxelTree::new();
    by_fill.fill_box(block, MaterialId(6)).unwrap();
    assert!(same_content(&by_voxel, &by_fill));
    assert_eq!(encode(&by_voxel), encode(&by_fill));
    // Live storage only: free lists depend on history, content does not.
    assert_eq!(
        NodeCounts::from(by_voxel.stats()),
        NodeCounts::from(by_fill.stats())
    );
}

#[test]
fn sixty_four_uniform_blocks_collapse_into_one_child() {
    let corner = VoxelPos::new(16, -32, 0);
    let mut blocks = VoxelTree::new();
    for i in 0..64 {
        let [x, y, z] = path::slot_offset(i).map(|v| v as i32 * 4);
        let block = VoxelBox::from_min_size(
            VoxelPos::new(corner.x + x, corner.y + y, corner.z + z),
            [4; 3],
        );
        blocks.fill_box(block, MaterialId(3)).unwrap();
    }
    assert!(matches!(
        child_at(&blocks, corner, 2),
        NodeRef::Uniform(MaterialId(3))
    ));
    let mut whole = VoxelTree::new();
    whole
        .fill_box(VoxelBox::from_min_size(corner, [16; 3]), MaterialId(3))
        .unwrap();
    assert_eq!(encode(&blocks), encode(&whole));
    assert_eq!(
        NodeCounts::from(blocks.stats()),
        NodeCounts::from(whole.stats())
    );
}

#[test]
fn breaking_and_restoring_a_uniform_region_collapses_it_again() {
    let corner = VoxelPos::new(-16, -16, -16);
    let region = VoxelBox::from_min_size(corner, [16; 3]);
    let mut tree = VoxelTree::new();
    tree.fill_box(region, MaterialId(5)).unwrap();
    let whole = NodeCounts::from(tree.stats());
    let bytes = encode(&tree);

    let inside = VoxelPos::new(-9, -3, -14);
    assert_eq!(tree.set(inside, MaterialId(7)), Ok(true));
    let split = check_canonical(&tree).unwrap();
    assert_eq!(split, NodeCounts::from(tree.stats()));
    // The brick is now a branch over 63 uniform children and one leaf.
    assert_eq!(split.branches, whole.branches + 1);
    assert_eq!(split.leaves, 1);

    assert_eq!(tree.set(inside, MaterialId(5)), Ok(true));
    assert_eq!(NodeCounts::from(tree.stats()), whole);
    assert_eq!(encode(&tree), bytes);
}
