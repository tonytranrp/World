//! The reference model is only useful if it is itself right. These tests check
//! it against structurally different implementations.

use std::collections::HashMap;

use proptest::prelude::*;
use world_core::{MaterialId, VoxelPos};
use world_testkit::dense::{
    Bounds, DenseWorld, VoxelModel, assert_models_agree, brute_force_cast, compare_after_edits,
};
use world_testkit::profile::{proptest_config, scale};
use world_testkit::strategies::{edits, ray_near};

fn bounds() -> Bounds {
    Bounds::new(VoxelPos::new(-3, -2, 1), [6, 5, 7])
}

/// The simplest possible model: a map of non-air voxels.
#[derive(Default)]
struct NaiveWorld(HashMap<VoxelPos, MaterialId>);

impl VoxelModel for NaiveWorld {
    fn get(&self, pos: VoxelPos) -> MaterialId {
        self.0.get(&pos).copied().unwrap_or(MaterialId::AIR)
    }

    fn set(&mut self, pos: VoxelPos, material: MaterialId) {
        if material.is_air() {
            self.0.remove(&pos);
        } else {
            self.0.insert(pos, material);
        }
    }
}

proptest! {
    #![proptest_config(proptest_config())]

    /// After any edit sequence the dense array and the map hold the same world.
    #[test]
    fn dense_model_matches_a_hash_map(list in edits(bounds(), 7, scale().edits_per_case)) {
        let mut dense = DenseWorld::new(bounds());
        let mut naive = NaiveWorld::default();
        prop_assert_eq!(compare_after_edits(&mut dense, &mut naive, &list, bounds()), Ok(()));
        prop_assert_eq!(dense.solid_count(), naive.0.len());
    }

    /// Setting a voxel is idempotent and the last write wins.
    #[test]
    fn last_write_wins(list in edits(bounds(), 7, scale().edits_per_case)) {
        let mut dense = DenseWorld::new(bounds());
        for e in &list {
            dense.apply(*e);
        }
        for e in &list {
            let last = list.iter().rev().find(|x| x.pos == e.pos).unwrap();
            prop_assert_eq!(dense.get(e.pos), last.material);
        }
    }

    /// The DDA traversal returns exactly what a slab test against every voxel returns.
    #[test]
    fn ray_traversal_matches_brute_force(
        list in edits(bounds(), 7, scale().edits_per_case),
        ray in ray_near(bounds()),
    ) {
        let mut dense = DenseWorld::new(bounds());
        for e in &list {
            dense.apply(*e);
        }
        prop_assert_eq!(dense.cast_ray(&ray), brute_force_cast(&dense, &ray));
    }
}

/// The plan's done-criterion for the voxel core: exactly `cases * edits_per_case`
/// random edits (one million in the full profile) with no divergence from the
/// reference. Proptest lengths vary and shrink; this test applies a fixed count.
#[test]
fn the_plans_edit_budget_never_diverges() {
    let s = scale();
    let b = bounds();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    let mut applied = 0usize;
    for _ in 0..s.cases {
        let (mut dense, mut naive) = (DenseWorld::new(b), NaiveWorld::default());
        let list: Vec<_> = (0..s.edits_per_case)
            .map(|_| {
                let [sx, sy, sz] = b.size;
                let pos = VoxelPos::new(
                    b.min.x + (next() % sx as u64) as i32,
                    b.min.y + (next() % sy as u64) as i32,
                    b.min.z + (next() % sz as u64) as i32,
                );
                world_core::Edit::new(pos, MaterialId((next() % 8) as u16))
            })
            .collect();
        assert_eq!(
            compare_after_edits(&mut dense, &mut naive, &list, b),
            Ok(())
        );
        applied += list.len();
    }
    assert_eq!(applied, s.cases as usize * s.edits_per_case);
}

#[test]
fn identical_worlds_agree_everywhere() {
    let (a, b) = (DenseWorld::new(bounds()), DenseWorld::new(bounds()));
    assert_eq!(assert_models_agree(&a, &b, bounds()), Ok(()));
}
