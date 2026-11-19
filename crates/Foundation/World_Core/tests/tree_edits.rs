//! The edit API's promises: batches are all-or-nothing, the dirty set is
//! exactly the bricks whose content changed, no-op edits report nothing,
//! `set` and `fill_box` say whether anything changed, and writes outside the
//! world fail without touching the tree.

use proptest::prelude::*;
use proptest::sample::Index;
use world_core::consts::{WORLD_END_VOXEL, WORLD_MIN_VOXEL};
use world_core::serialize::encode;
use world_core::tree::{NodeRef, same_content};
use world_core::{
    DirtyBricks, Edit, EditError, MaterialId, TreeRead, VoxelBox, VoxelPos, VoxelTree,
};
use world_testkit::dense::{
    Bounds, DenseWorld, EditOp, NodeCounts, VoxelModel, assert_tree_agrees, check_canonical,
};
use world_testkit::profile::{proptest_config, scale};
use world_testkit::strategies::{edit_ops, edits, material, voxel_box};

/// Crosses the origin and spans several bricks on every axis.
const B: Bounds = Bounds::new(VoxelPos::new(-18, -7, -25), [36, 20, 40]);

fn build(ops: &[EditOp]) -> (VoxelTree, DenseWorld) {
    let (mut tree, mut dense) = (VoxelTree::new(), DenseWorld::new(B));
    for &op in ops {
        tree.apply_op(op);
        dense.apply_op(op);
    }
    (tree, dense)
}

/// The bricks holding a voxel whose material differs between the two models.
fn changed_bricks(before: &DenseWorld, after: &DenseWorld) -> DirtyBricks {
    let changed = before
        .solids()
        .chain(after.solids())
        .map(|(p, _)| p)
        .filter(|&p| before.get(p) != after.get(p))
        .map(VoxelPos::brick)
        .collect();
    DirtyBricks::from_unsorted(changed)
}

/// A position with at least one axis outside the world, possibly far outside.
fn outside_pos() -> impl Strategy<Value = VoxelPos> {
    let inside = || WORLD_MIN_VOXEL..WORLD_END_VOXEL;
    let outside = || {
        prop_oneof![
            (WORLD_END_VOXEL..WORLD_END_VOXEL + 4),
            (WORLD_MIN_VOXEL - 4..WORLD_MIN_VOXEL),
            (WORLD_END_VOXEL..=i32::MAX),
            (i32::MIN..WORLD_MIN_VOXEL),
        ]
    };
    ((inside(), inside(), inside()), outside(), 0..3usize).prop_map(|((x, y, z), out, axis)| {
        let mut v = [x, y, z];
        v[axis] = out;
        VoxelPos::from(v)
    })
}

/// A non-empty box that crosses one face of the world.
fn straddling_box() -> impl Strategy<Value = VoxelBox> {
    (
        1i32..6,
        1u32..10,
        0usize..3,
        any::<bool>(),
        (-8i32..8, -8i32..8),
    )
        .prop_map(|(inside, extra, axis, high, (a, b))| {
            let mut min = [a, b, a - b];
            let mut size = [4u32; 3];
            min[axis] = if high {
                WORLD_END_VOXEL - inside
            } else {
                WORLD_MIN_VOXEL - extra as i32
            };
            size[axis] = inside as u32 + extra;
            VoxelBox::from_min_size(VoxelPos::from(min), size)
        })
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn apply_reports_exactly_the_bricks_whose_content_changed(
        setup in edit_ops(B, 3, scale().edits_per_case),
        batch in edits(B, 3, scale().edits_per_case),
    ) {
        let (mut tree, mut dense) = build(&setup);
        let before = dense.clone();
        for &e in &batch {
            dense.apply(e);
        }
        prop_assert_eq!(tree.apply(&batch), Ok(changed_bricks(&before, &dense)));
        prop_assert_eq!(assert_tree_agrees(&tree, &dense, B), Ok(()));
        prop_assert!(check_canonical(&tree).is_ok());
    }

    /// Applying a batch again leaves every voxel as it was: nothing to report.
    #[test]
    fn repeating_a_batch_reports_nothing(
        setup in edit_ops(B, 3, scale().edits_per_case),
        batch in edits(B, 3, scale().edits_per_case),
    ) {
        let (mut tree, _) = build(&setup);
        tree.apply(&batch).unwrap();
        let bytes = encode(&tree);
        prop_assert_eq!(tree.apply(&batch), Ok(DirtyBricks::default()));
        prop_assert_eq!(encode(&tree), bytes);
    }

    #[test]
    fn a_batch_with_an_out_of_world_edit_changes_nothing(
        setup in edit_ops(B, 3, scale().edits_per_case),
        batch in edits(B, 3, scale().edits_per_case),
        bad in outside_pos(),
        at in any::<Index>(),
    ) {
        let (mut tree, _) = build(&setup);
        let (before, bytes) = (tree.snapshot(), encode(&tree));
        let mut batch = batch;
        batch.insert(at.index(batch.len() + 1), Edit::new(bad, MaterialId(1)));

        let err = tree.apply(&batch).unwrap_err();
        prop_assert_eq!(err, EditError::OutOfWorld { pos: bad });
        let named = format!("({}, {}, {})", bad.x, bad.y, bad.z);
        prop_assert!(err.to_string().contains(&named));
        prop_assert!(same_content(&tree, &before));
        prop_assert_eq!(encode(&tree), bytes);
    }

    #[test]
    fn set_reports_whether_the_voxel_changed(
        setup in edit_ops(B, 3, scale().edits_per_case),
        list in edits(B, 3, scale().edits_per_case),
    ) {
        let (mut tree, mut dense) = build(&setup);
        for e in list {
            let changes = dense.get(e.pos) != e.material;
            prop_assert_eq!(tree.set(e.pos, e.material), Ok(changes));
            dense.apply(e);
        }
        prop_assert_eq!(assert_tree_agrees(&tree, &dense, B), Ok(()));
    }

    #[test]
    fn fill_box_reports_whether_anything_changed(
        setup in edit_ops(B, 3, scale().edits_per_case),
        region in voxel_box(B),
        m in material(3),
    ) {
        let (mut tree, mut dense) = build(&setup);
        let before = dense.clone();
        dense.fill_box(region, m);
        let changes = !changed_bricks(&before, &dense).is_empty();
        prop_assert_eq!(tree.fill_box(region, m), Ok(changes));
        prop_assert_eq!(assert_tree_agrees(&tree, &dense, B), Ok(()));
        prop_assert!(check_canonical(&tree).is_ok());
    }

    #[test]
    fn writes_outside_the_world_fail_and_change_nothing(
        setup in edit_ops(B, 3, scale().edits_per_case),
        bad in outside_pos(),
        region in straddling_box(),
    ) {
        // Content in the box's in-world part, which a partial fill would overwrite.
        let (mut tree, _) = build(&setup);
        tree.fill_box(region.intersection(&VoxelBox::WORLD), MaterialId(2)).unwrap();
        let bytes = encode(&tree);

        prop_assert_eq!(tree.set(bad, MaterialId(1)), Err(EditError::OutOfWorld { pos: bad }));
        match tree.fill_box(region, MaterialId(3)) {
            Err(EditError::OutOfWorld { pos }) => {
                prop_assert!(region.contains(pos) && !pos.in_world(), "{:?}", pos);
            }
            other => prop_assert!(false, "fill of {:?} gave {:?}", region, other),
        }
        prop_assert_eq!(encode(&tree), bytes);
    }
}

#[test]
fn an_edit_undone_in_the_same_batch_reports_nothing() {
    let mut tree = VoxelTree::new();
    let p = VoxelPos::new(-3, 5, 17);
    tree.set(p, MaterialId(2)).unwrap();
    let bytes = encode(&tree);
    let batch = [
        Edit::new(p, MaterialId(4)),
        Edit::new(VoxelPos::new(40, 0, 0), MaterialId(1)),
        Edit::new(VoxelPos::new(40, 0, 0), MaterialId::AIR),
        Edit::new(p, MaterialId(2)),
    ];
    assert_eq!(tree.apply(&batch), Ok(DirtyBricks::default()));
    assert_eq!(encode(&tree), bytes);
}

#[test]
fn no_op_edits_report_nothing() {
    let mut tree = VoxelTree::new();
    let p = VoxelPos::new(7, -7, 0);
    assert_eq!(tree.apply(&[]), Ok(DirtyBricks::default()));
    assert_eq!(tree.set(p, MaterialId::AIR), Ok(false));
    assert_eq!(
        tree.apply(&[Edit::new(p, MaterialId::AIR)]),
        Ok(DirtyBricks::default())
    );
    tree.set(p, MaterialId(5)).unwrap();
    assert_eq!(tree.set(p, MaterialId(5)), Ok(false));
    assert_eq!(
        tree.apply(&[Edit::new(p, MaterialId(5))]),
        Ok(DirtyBricks::default())
    );
    let region = VoxelBox::from_min_size(VoxelPos::new(-16, 0, 0), [16; 3]);
    tree.fill_box(region, MaterialId(1)).unwrap();
    assert_eq!(tree.fill_box(region, MaterialId(1)), Ok(false));
}

#[test]
fn an_empty_fill_is_a_no_op_anywhere() {
    let mut tree = VoxelTree::new();
    let far = VoxelPos::new(WORLD_END_VOXEL + 100, 0, i32::MIN);
    for region in [
        VoxelBox::new(VoxelPos::ORIGIN, VoxelPos::ORIGIN),
        VoxelBox::new(VoxelPos::new(0, 0, 0), VoxelPos::new(5, -5, 5)),
        VoxelBox::new(far, far),
        VoxelBox::from_min_size(far, [0, 10, 10]),
    ] {
        assert_eq!(
            tree.fill_box(region, MaterialId(3)),
            Ok(false),
            "{region:?}"
        );
    }
    assert!(matches!(tree.root(), NodeRef::Air));
    assert_eq!(NodeCounts::from(tree.stats()), NodeCounts::default());
}

#[test]
fn the_whole_world_fills_and_clears_in_one_step() {
    let mut tree = VoxelTree::new();
    assert_eq!(tree.fill_box(VoxelBox::WORLD, MaterialId(9)), Ok(true));
    assert!(matches!(tree.root(), NodeRef::Uniform(MaterialId(9))));
    let corner = VoxelBox::WORLD.min();
    assert_eq!(TreeRead::get(&tree, corner), MaterialId(9));

    // One voxel of air at a corner needs a path of branches down to one leaf.
    assert_eq!(tree.set(corner, MaterialId::AIR), Ok(true));
    let counts = check_canonical(&tree).unwrap();
    assert_eq!(counts, NodeCounts::from(tree.stats()));
    assert_eq!((counts.branches, counts.leaves), (9, 1));

    assert_eq!(tree.fill_box(VoxelBox::WORLD, MaterialId::AIR), Ok(true));
    assert!(matches!(tree.root(), NodeRef::Air));
    assert_eq!(NodeCounts::from(tree.stats()), NodeCounts::default());
    assert_eq!(tree.fill_box(VoxelBox::WORLD, MaterialId::AIR), Ok(false));
}

#[test]
fn edits_at_the_world_corners_dirty_the_corner_bricks() {
    let (lo, hi) = (WORLD_MIN_VOXEL, WORLD_END_VOXEL - 1);
    let corners: Vec<VoxelPos> = (0..8)
        .map(|i| {
            let pick = |bit: i32| if i >> bit & 1 == 1 { hi } else { lo };
            VoxelPos::new(pick(0), pick(1), pick(2))
        })
        .collect();
    let mut tree = VoxelTree::new();
    let batch: Vec<Edit> = corners
        .iter()
        .map(|&p| Edit::new(p, MaterialId(1)))
        .collect();
    let expected = DirtyBricks::from_unsorted(corners.iter().map(|p| p.brick()).collect());
    assert_eq!(expected.len(), 8);
    assert_eq!(tree.apply(&batch), Ok(expected.clone()));
    for &p in &corners {
        assert_eq!(TreeRead::get(&tree, p), MaterialId(1));
    }

    let clear: Vec<Edit> = corners
        .iter()
        .map(|&p| Edit::new(p, MaterialId::AIR))
        .collect();
    assert_eq!(tree.apply(&clear), Ok(expected));
    assert!(matches!(tree.root(), NodeRef::Air));
}

/// Writing a box voxel by voxel through `apply` reports exactly the bricks
/// the box touches, and gives the same tree as `fill_box`.
#[test]
fn apply_and_fill_box_agree_on_a_box() {
    let region = VoxelBox::from_min_size(VoxelPos::new(-20, -1, 15), [21, 2, 3]);
    let mut by_apply = VoxelTree::new();
    let mut batch = Vec::new();
    let mut bricks = Vec::new();
    for z in region.min().z..region.end().z {
        for y in region.min().y..region.end().y {
            for x in region.min().x..region.end().x {
                let p = VoxelPos::new(x, y, z);
                batch.push(Edit::new(p, MaterialId(2)));
                bricks.push(p.brick());
            }
        }
    }
    assert_eq!(
        by_apply.apply(&batch),
        Ok(DirtyBricks::from_unsorted(bricks))
    );
    let mut by_fill = VoxelTree::new();
    assert_eq!(by_fill.fill_box(region, MaterialId(2)), Ok(true));
    assert!(same_content(&by_apply, &by_fill));
}
