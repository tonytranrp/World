//! Dense object grids against the dense reference model: random voxel edits
//! and box fills at every grid size, with the canonical view checked after
//! each sequence, and reads through the shared `VoxelSource` interface.

use proptest::prelude::*;
use world_core::tree::{DenseGrid, NodeKind, NodeView, VoxelSource};
use world_core::{MaterialId, VoxelPos};
use world_testkit::dense::{
    Bounds, DenseWorld, NodeCounts, assert_tree_agrees, check_canonical, compare_after_ops,
};
use world_testkit::profile::{proptest_config, scale};
use world_testkit::strategies::edit_ops;

/// The grid's own cube as model bounds.
fn bounds(grid: &DenseGrid) -> Bounds {
    Bounds::new(grid.origin(), [grid.size(); 3])
}

/// The grid's cube grown by one voxel on every side, to catch reads that
/// leak outside it.
fn around(grid: &DenseGrid) -> Bounds {
    let o = grid.origin();
    Bounds::new(
        VoxelPos::new(o.x - 1, o.y - 1, o.z - 1),
        [grid.size() + 2; 3],
    )
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn grids_match_the_dense_model_at_every_size(
        (levels, ops) in (1u32..=3).prop_flat_map(|levels| {
            let size = 1u32 << (2 * levels);
            let b = Bounds::new(VoxelPos::new(-7, 40, -3), [size; 3]);
            (Just(levels), edit_ops(b, 5, scale().edits_per_case))
        }),
    ) {
        let mut grid = DenseGrid::new(levels, VoxelPos::new(-7, 40, -3));
        let cube = bounds(&grid);
        let mut dense = DenseWorld::new(cube);
        prop_assert_eq!(compare_after_ops(&mut grid, &mut dense, &ops, cube), Ok(()));
        prop_assert_eq!(assert_tree_agrees(&grid, &dense, around(&grid)), Ok(()));
        check_canonical(&grid).map_err(TestCaseError::fail)?;
    }
}

#[test]
fn an_empty_and_a_full_grid_hold_no_nodes() {
    let mut grid = DenseGrid::new(2, VoxelPos::ORIGIN);
    assert_eq!(check_canonical(&grid), Ok(NodeCounts::default()));
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                grid.set(VoxelPos::new(x, y, z), MaterialId(4));
            }
        }
    }
    assert_eq!(grid.root().kind(), NodeKind::Uniform(MaterialId(4)));
    assert_eq!(check_canonical(&grid), Ok(NodeCounts::default()));
}

#[test]
fn one_voxel_is_one_branch_and_one_leaf() {
    let mut grid = DenseGrid::new(2, VoxelPos::new(100, 100, 100));
    assert!(grid.set(VoxelPos::new(105, 101, 115), MaterialId(9)));
    let counts = check_canonical(&grid).unwrap();
    assert_eq!((counts.branches, counts.leaves), (1, 1));
    // As the world tree would store it: a brick header and one three-word
    // record, no palette.
    assert_eq!(counts.words, 6);
    assert_eq!(
        VoxelSource::get(&grid, VoxelPos::new(105, 101, 115)),
        MaterialId(9)
    );
    assert_eq!(grid.get(VoxelPos::new(99, 101, 115)), MaterialId::AIR);
}
