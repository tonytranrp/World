//! Bulk building (decision 0006): a tree built bottom-up from a region
//! source holds exactly the source's content, in canonical form, and a
//! rebuilt region replaces exactly its node's cube.

use proptest::prelude::*;
use world_core::consts::{TREE_DEPTH, WORLD_MIN_VOXEL};
use world_core::tree::same_content;
use world_core::{EditError, MaterialId, RegionSource, VoxelPos, VoxelTree};
use world_testkit::dense::{
    Bounds, DenseWorld, EditOp, NodeCounts, VoxelModel, assert_tree_agrees, check_canonical,
};
use world_testkit::profile::proptest_config;
use world_testkit::strategies::edit_ops;

/// Crosses the origin and spans several bricks and a level-2 node boundary.
const B: Bounds = Bounds::new(VoxelPos::new(-40, -9, -70), [80, 30, 90]);

/// A dense model read as a region source. Cubes outside the model are air.
/// For cubes that overlap it, `exact` answers truthfully by scanning;
/// otherwise the answer is always "unknown", which forces the builder down to
/// every leaf of the model.
struct Dense<'a> {
    world: &'a DenseWorld,
    exact: bool,
}

impl RegionSource for Dense<'_> {
    fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
        let b = self.world.bounds();
        let lo = b.min.to_array();
        let m = min.to_array();
        let overlap: Vec<(i64, i64)> = (0..3)
            .map(|a| {
                let start = i64::from(m[a]).max(i64::from(lo[a]));
                let end = (i64::from(m[a]) + i64::from(size))
                    .min(i64::from(lo[a]) + i64::from(b.size[a]));
                (start, end)
            })
            .collect();
        if overlap.iter().any(|&(s, e)| s >= e) {
            return Some(MaterialId::AIR);
        }
        if !self.exact {
            return None;
        }
        let inside = overlap.iter().all(|&(s, e)| e - s == i64::from(size));
        let first = self.world.get(VoxelPos::new(
            overlap[0].0 as i32,
            overlap[1].0 as i32,
            overlap[2].0 as i32,
        ));
        if !inside && !first.is_air() {
            return None;
        }
        for z in overlap[2].0..overlap[2].1 {
            for y in overlap[1].0..overlap[1].1 {
                for x in overlap[0].0..overlap[0].1 {
                    if self.world.get(VoxelPos::new(x as i32, y as i32, z as i32)) != first {
                        return None;
                    }
                }
            }
        }
        Some(first)
    }

    fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        for (slot, voxel) in (0..).zip(voxels.iter_mut()) {
            *voxel = self.world.get(VoxelPos::new(
                min.x + slot % 4,
                min.y + slot / 4 % 4,
                min.z + slot / 16,
            ));
        }
    }
}

fn dense_from(ops: &[EditOp]) -> DenseWorld {
    let mut dense = DenseWorld::new(B);
    for &op in ops {
        dense.apply_op(op);
    }
    dense
}

fn tree_from(ops: &[EditOp]) -> VoxelTree {
    let mut tree = VoxelTree::new();
    for &op in ops {
        tree.apply_op(op);
    }
    tree
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn a_built_tree_equals_the_same_content_written_voxel_by_voxel(
        ops in edit_ops(B, 5, 40),
        exact in any::<bool>(),
    ) {
        let dense = dense_from(&ops);
        let built = VoxelTree::from_source(&Dense { world: &dense, exact });
        let written = tree_from(&ops);
        prop_assert!(same_content(&built, &written));
        let counts = check_canonical(&built).map_err(TestCaseError::fail)?;
        prop_assert_eq!(counts, NodeCounts::from(written.stats()));
        prop_assert_eq!(NodeCounts::from(built.stats()), NodeCounts::from(written.stats()));
        prop_assert_eq!(built.stats().free_nodes, 0, "a fresh build frees no node");
        prop_assert!(assert_tree_agrees(&built, &dense, B).is_ok());
    }

    #[test]
    fn rebuilding_a_region_replaces_exactly_that_node(
        base in edit_ops(B, 5, 30),
        patch in edit_ops(B, 5, 30),
        level in 1u32..=2,
        corner in 0usize..8,
    ) {
        let (mut tree, mut expected) = (tree_from(&base), dense_from(&base));
        let source_world = dense_from(&patch);
        // A node of `level` inside the bounds, chosen by `corner`.
        let size = 4i32.pow(level + 1);
        let pick = |a: usize, bit: usize| {
            let lo = B.min.to_array()[a];
            let first = (lo - WORLD_MIN_VOXEL + size - 1) / size * size + WORLD_MIN_VOXEL;
            first + if corner >> bit & 1 == 1 { size } else { 0 }
        };
        let min = VoxelPos::new(pick(0, 0), pick(1, 1), pick(2, 2));
        tree.build_region(level, min, &Dense { world: &source_world, exact: true })
            .unwrap();
        for z in 0..size {
            for y in 0..size {
                for x in 0..size {
                    let p = VoxelPos::new(min.x + x, min.y + y, min.z + z);
                    if B.contains(p) {
                        expected.set(p, source_world.get(p));
                    } else {
                        prop_assert_eq!(tree.get(p), source_world.get(p));
                    }
                }
            }
        }
        prop_assert!(assert_tree_agrees(&tree, &expected, B).is_ok());
        check_canonical(&tree).map_err(TestCaseError::fail)?;
    }
}

#[test]
fn whole_node_operations_check_their_position() {
    let mut tree = VoxelTree::new();
    let world = DenseWorld::new(B);
    let source = Dense {
        world: &world,
        exact: true,
    };
    let off = VoxelPos::new(1, 0, 0);
    assert_eq!(
        tree.build_region(1, off, &source),
        Err(EditError::Misaligned { pos: off, level: 1 })
    );
    let origin = VoxelPos::new(0, 0, 0);
    assert_eq!(
        tree.build_region(0, origin, &source),
        Err(EditError::Misaligned {
            pos: origin,
            level: 0
        })
    );
    assert_eq!(
        tree.build_region(TREE_DEPTH, origin, &source),
        Err(EditError::Misaligned {
            pos: origin,
            level: TREE_DEPTH
        })
    );
    let outside = VoxelPos::new(WORLD_MIN_VOXEL - 16, 0, 0);
    assert_eq!(
        tree.build_region(1, outside, &source),
        Err(EditError::OutOfWorld { pos: outside })
    );
    assert_eq!(tree.build_region(1, origin, &source), Ok(false));
}

#[test]
fn rebuilding_a_region_as_air_frees_its_nodes() {
    let mut tree = VoxelTree::new();
    tree.set(VoxelPos::new(3, 3, 3), MaterialId(4)).unwrap();
    let empty = DenseWorld::new(B);
    let source = Dense {
        world: &empty,
        exact: true,
    };
    assert_eq!(
        tree.build_region(1, VoxelPos::new(0, 0, 0), &source),
        Ok(true)
    );
    let s = tree.stats();
    assert_eq!((s.branches, s.leaves, s.words), (0, 0, 0));
}
