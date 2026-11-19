//! Coordinate math against naive references: floor division written out by
//! hand, slot digits computed by division instead of shifts, and boxes
//! checked point by point. Negative values and the world edges are generated
//! on purpose.

use proptest::prelude::*;
use world_core::consts::{
    BRICK_DIM, TREE_BRANCH_DIM, TREE_DEPTH, WORLD_END_VOXEL, WORLD_MIN_VOXEL, WORLD_SIZE_VOXELS,
};
use world_core::coords::path;
use world_core::{BrickPos, VoxelBox, VoxelPos};
use world_testkit::profile::proptest_config;

/// Floor division by sign analysis, without `div_euclid`.
fn floor_div(a: i64, b: i64) -> i64 {
    let q = a / b;
    if a % b != 0 && (a < 0) != (b < 0) {
        q - 1
    } else {
        q
    }
}

/// Any `i32`, with values at and around the world edges and zero favoured.
fn coord() -> impl Strategy<Value = i32> {
    prop_oneof![
        any::<i32>(),
        (WORLD_MIN_VOXEL - 40)..(WORLD_MIN_VOXEL + 40),
        (WORLD_END_VOXEL - 40)..(WORLD_END_VOXEL + 40),
        -40i32..40,
        Just(i32::MIN),
        Just(i32::MAX),
    ]
}

fn pos() -> impl Strategy<Value = VoxelPos> {
    (coord(), coord(), coord()).prop_map(|(x, y, z)| VoxelPos::new(x, y, z))
}

/// A coordinate inside the world, edges favoured.
fn world_coord() -> impl Strategy<Value = i32> {
    prop_oneof![
        WORLD_MIN_VOXEL..WORLD_END_VOXEL,
        WORLD_MIN_VOXEL..(WORLD_MIN_VOXEL + 40),
        (WORLD_END_VOXEL - 40)..WORLD_END_VOXEL,
        -40i32..40,
    ]
}

fn world_pos() -> impl Strategy<Value = VoxelPos> {
    (world_coord(), world_coord(), world_coord()).prop_map(|(x, y, z)| VoxelPos::new(x, y, z))
}

fn small_box() -> impl Strategy<Value = VoxelBox> {
    (
        (-6i32..6, -6i32..6, -6i32..6),
        (-2i32..8, -2i32..8, -2i32..8),
    )
        .prop_map(|((x, y, z), (dx, dy, dz))| {
            VoxelBox::new(
                VoxelPos::new(x, y, z),
                VoxelPos::new(x + dx, y + dy, z + dz),
            )
        })
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn bricks_floor_divide_every_axis(p in pos()) {
        let b = BrickPos::containing(p);
        let d = BRICK_DIM as i64;
        prop_assert_eq!(
            [b.x, b.y, b.z].map(i64::from),
            p.to_array().map(|v| floor_div(v.into(), d))
        );
        prop_assert_eq!(p.brick(), b);
    }

    /// A brick's bounds hold exactly the voxels that map to it (inside the
    /// world, where its end fits an `i32`).
    #[test]
    fn a_brick_holds_exactly_its_voxels(p in world_pos(), q in world_pos()) {
        let b = p.brick();
        let bounds = b.bounds();
        prop_assert!(bounds.contains(p));
        prop_assert_eq!(bounds.min(), b.min_voxel());
        prop_assert_eq!(bounds.size(), [BRICK_DIM; 3]);
        prop_assert_eq!(bounds.contains(q), q.brick() == b);
    }

    #[test]
    fn world_membership_and_local_coordinates(p in pos()) {
        let inside = p.to_array().iter().all(|&v| (WORLD_MIN_VOXEL..WORLD_END_VOXEL).contains(&v));
        prop_assert_eq!(p.in_world(), inside);
        match path::world_local(p) {
            Some(local) => {
                prop_assert!(inside);
                let expected = p.to_array().map(|v| (i64::from(v) - i64::from(WORLD_MIN_VOXEL)) as u32);
                prop_assert_eq!(local, expected);
                prop_assert!(local.iter().all(|&l| l < WORLD_SIZE_VOXELS));
                prop_assert_eq!(path::from_world_local(local), p);
            }
            None => prop_assert!(!inside),
        }
    }

    /// The slot at each level is the base-4 digit of each local axis, and the
    /// digits of all levels put back together give the position again.
    #[test]
    fn slot_paths_are_base_four_digits(p in world_pos()) {
        let local = path::world_local(p).unwrap();
        let mut rebuilt = [0u32; 3];
        for level in 0..TREE_DEPTH {
            let span = TREE_BRANCH_DIM.pow(level);
            let digit = local.map(|v| (v / span) % TREE_BRANCH_DIM);
            let slot = path::child_slot(local, level);
            prop_assert_eq!(slot, digit[0] + 4 * digit[1] + 16 * digit[2]);
            prop_assert!(slot < 64);
            prop_assert_eq!(path::slot_offset(slot), digit);
            for (sum, offset) in rebuilt.iter_mut().zip(path::slot_offset(slot)) {
                *sum += offset * path::child_span(level);
            }
        }
        prop_assert_eq!(path::from_world_local(rebuilt), p);
    }

    /// A brick is the region of one level-1 node: voxels share a brick exactly
    /// when their slot paths agree from the root down to level 2.
    #[test]
    fn bricks_are_level_one_nodes(p in world_pos(), q in world_pos()) {
        let (lp, lq) = (path::world_local(p).unwrap(), path::world_local(q).unwrap());
        let same_path = (2..TREE_DEPTH).all(|l| path::child_slot(lp, l) == path::child_slot(lq, l));
        prop_assert_eq!(p.brick() == q.brick(), same_path);
        let b = p.brick();
        prop_assert_eq!(b.x, (lp[0] / BRICK_DIM) as i32 + WORLD_MIN_VOXEL / BRICK_DIM as i32);
    }

    #[test]
    fn boxes_agree_with_a_point_by_point_reference(a in small_box(), b in small_box()) {
        let points = || {
            (-8..14).flat_map(|z| (-8..14).flat_map(move |y| (-8..14).map(move |x| VoxelPos::new(x, y, z))))
        };
        let inside = |bx: &VoxelBox, p: VoxelPos| {
            let (lo, end) = (bx.min().to_array(), bx.end().to_array());
            (0..3).all(|i| lo[i] <= p.to_array()[i] && p.to_array()[i] < end[i])
        };
        let count = points().filter(|&p| inside(&a, p)).count() as u64;
        prop_assert_eq!(a.volume(), count);
        prop_assert_eq!(a.is_empty(), count == 0);
        let i = a.intersection(&b);
        for p in points() {
            prop_assert_eq!(a.contains(p), inside(&a, p));
            prop_assert_eq!(i.contains(p), inside(&a, p) && inside(&b, p));
        }
        let all_in = points().filter(|&p| inside(&b, p)).all(|p| inside(&a, p));
        prop_assert_eq!(a.contains_box(&b), all_in);
    }

    #[test]
    fn from_min_size_spans_the_size(p in world_pos(), size in (0u32..40, 0u32..40, 0u32..40)) {
        let size = [size.0, size.1, size.2];
        let b = VoxelBox::from_min_size(p, size);
        prop_assert_eq!(b.min(), p);
        prop_assert_eq!(b.size(), size);
        prop_assert_eq!(b.volume(), size.iter().map(|&s| u64::from(s)).product::<u64>());
    }
}

#[test]
fn child_spans_are_powers_of_four() {
    let mut span = 1;
    for level in 0..=TREE_DEPTH {
        assert_eq!(path::child_span(level), span, "level {level}");
        span *= 4;
    }
    assert_eq!(path::child_span(TREE_DEPTH), WORLD_SIZE_VOXELS);
}

#[test]
fn world_corners_have_extreme_paths() {
    let low = VoxelPos::new(WORLD_MIN_VOXEL, WORLD_MIN_VOXEL, WORLD_MIN_VOXEL);
    let high = VoxelPos::new(
        WORLD_END_VOXEL - 1,
        WORLD_END_VOXEL - 1,
        WORLD_END_VOXEL - 1,
    );
    for level in 0..TREE_DEPTH {
        assert_eq!(path::child_slot(path::world_local(low).unwrap(), level), 0);
        assert_eq!(
            path::child_slot(path::world_local(high).unwrap(), level),
            63
        );
    }
    // The origin is the first voxel of the upper half on every axis.
    let origin = path::world_local(VoxelPos::ORIGIN).unwrap();
    assert_eq!(path::child_slot(origin, TREE_DEPTH - 1), 2 + 4 * 2 + 16 * 2);
    for level in 0..TREE_DEPTH - 1 {
        assert_eq!(path::child_slot(origin, level), 0);
    }
}

#[test]
fn the_world_box_is_exactly_the_world() {
    let w = VoxelBox::WORLD;
    assert_eq!(w.volume(), u64::from(WORLD_SIZE_VOXELS).pow(3));
    for p in [
        VoxelPos::new(WORLD_MIN_VOXEL, 0, 0),
        VoxelPos::new(0, WORLD_END_VOXEL - 1, 0),
    ] {
        assert!(w.contains(p) && p.in_world());
    }
    for p in [
        VoxelPos::new(WORLD_MIN_VOXEL - 1, 0, 0),
        VoxelPos::new(0, 0, WORLD_END_VOXEL),
    ] {
        assert!(!w.contains(p) && !p.in_world());
    }
}
