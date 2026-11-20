//! Whole-chunk operations (decision 0006): listing, compaction, region
//! copies, coarse summaries, edit overlays and the byte breakdown.

use proptest::prelude::*;
use world_core::tree::{CHUNK_LEVEL, MemoryBreakdown, TreeRead, representative, same_content};
use world_core::{MaterialId, VoxelPos, VoxelTree};
use world_testkit::dense::{
    Bounds, DenseWorld, EditOp, VoxelModel, assert_tree_agrees, check_canonical,
};
use world_testkit::profile::proptest_config;
use world_testkit::strategies::edit_ops;

/// Inside the chunk whose lowest voxel is the world centre, away from its
/// faces, spanning several bricks.
const B: Bounds = Bounds::new(VoxelPos::new(8, 8, 8), [70, 30, 70]);
const CHUNK: VoxelPos = VoxelPos::new(0, 0, 0);

fn world_of(ops: &[EditOp]) -> DenseWorld {
    let mut w = DenseWorld::new(B);
    for &op in ops {
        w.apply_op(op);
    }
    w
}

fn tree_of(world: &DenseWorld) -> VoxelTree {
    VoxelTree::from_source(world)
}

/// The words a tree's breakdown accounts for, which must be its live words.
fn breakdown_words(b: &MemoryBreakdown) -> usize {
    b.child_words
        + 3 * (b.full_records + b.single_records + b.palette_records)
        + b.palette_words.iter().sum::<usize>()
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn compaction_keeps_content_and_removes_holes(
        build in edit_ops(B, 6, 40),
        carve in edit_ops(B, 0, 40),
    ) {
        let mut tree = tree_of(&world_of(&build));
        let mut model = world_of(&build);
        for &op in &carve {
            tree.apply_op(op);
            model.apply_op(op);
        }
        let before = tree.snapshot();
        let compacted = tree.compact_chunk(CHUNK).unwrap();
        prop_assert_eq!(compacted, tree.chunks().len() == 1);
        prop_assert!(same_content(&tree, &before));
        prop_assert!(assert_tree_agrees(&tree, &model, B).is_ok());
        check_canonical(&tree).map_err(TestCaseError::fail)?;
        for chunk in tree.chunks() {
            prop_assert_eq!(chunk.stats.free_nodes, 0);
            // Only page tails remain free after a depth-first rebuild.
            prop_assert!(chunk.stats.free_words < chunk.stats.words / 4 + 200);
        }
    }

    #[test]
    fn an_overlay_rebuilds_the_edited_chunk_from_its_base(
        base_ops in edit_ops(B, 6, 30),
        edits in edit_ops(B, 6, 30),
    ) {
        let base = world_of(&base_ops);
        let mut edited = world_of(&base_ops);
        let mut tree = VoxelTree::unloaded();
        tree.build_region(CHUNK_LEVEL, CHUNK, &base).unwrap();
        for &op in &edits {
            tree.apply_op(op);
            edited.apply_op(op);
        }
        let overlay = tree.overlay(CHUNK, &base).unwrap();
        check_canonical(&overlay).map_err(TestCaseError::fail)?;

        // A fresh tree with nothing loaded gets the chunk back exactly.
        let mut restored = VoxelTree::unloaded();
        restored.apply_overlay(CHUNK, &base, &overlay).unwrap();
        prop_assert!(assert_tree_agrees(&restored, &edited, B).is_ok());
        let chunk = |t: &VoxelTree| t.extract(CHUNK_LEVEL, CHUNK).unwrap();
        prop_assert!(same_content(&chunk(&restored), &chunk(&tree)));
        check_canonical(&restored).map_err(TestCaseError::fail)?;

        // Changed voxels hold the new material; unchanged ones are KEEP, or
        // the same material where a whole region was overwritten at once
        // (one uniform word beats a mix of KEEP and air).
        for p in [B.min, VoxelPos::new(40, 20, 40), VoxelPos::new(77, 37, 77)] {
            let v = overlay.get(p);
            if edited.get(p) == base.get(p) {
                prop_assert!(v == MaterialId::KEEP || v == edited.get(p));
            } else {
                prop_assert_eq!(v, edited.get(p));
            }
        }
    }

    #[test]
    fn extraction_copies_exactly_one_region(ops in edit_ops(B, 6, 40)) {
        let world = world_of(&ops);
        let tree = tree_of(&world);
        // The level-2 node (64 voxels) holding B's lowest corner.
        let region = tree.extract(2, VoxelPos::new(0, 0, 0)).unwrap();
        for z in 0..80 {
            for x in (0..80).step_by(3) {
                for y in (0..40).step_by(2) {
                    let p = VoxelPos::new(x, y, z);
                    let inside = x < 64 && y < 64 && z < 64;
                    let want = if inside { world.get(p) } else { MaterialId::AIR };
                    prop_assert_eq!(region.get(p), want);
                }
            }
        }
        check_canonical(&region).map_err(TestCaseError::fail)?;
    }

    #[test]
    fn breakdown_accounts_for_every_live_word(ops in edit_ops(B, 20, 60)) {
        let tree = tree_of(&world_of(&ops));
        let b = tree.memory_breakdown();
        let s = tree.stats();
        prop_assert_eq!(b.upper_nodes + b.bricks, s.branches);
        prop_assert_eq!(breakdown_words(&b), s.words);
        prop_assert_eq!(b.single_records + b.palette_records, s.leaves);
        prop_assert_eq!(b.live_bytes(), 16 * s.branches + 4 * s.words);
    }
}

#[test]
fn chunks_are_listed_with_their_position_and_generation() {
    let mut tree = VoxelTree::new();
    assert!(tree.chunks().is_empty());
    tree.set(VoxelPos::new(5, 5, 5), MaterialId(1)).unwrap();
    tree.set(VoxelPos::new(-3, 7, 2000), MaterialId(2)).unwrap();
    let chunks = tree.chunks();
    let mins: Vec<VoxelPos> = chunks.iter().map(|c| c.min).collect();
    assert_eq!(
        mins,
        [VoxelPos::new(-1024, 0, 1024), VoxelPos::new(0, 0, 0)],
        "tree order: the lower slot path first"
    );
    let g = tree.chunk_generation(VoxelPos::new(5, 5, 5)).unwrap();
    assert_eq!(chunks[1].generation, g);
    tree.set(VoxelPos::new(6, 5, 5), MaterialId(1)).unwrap();
    assert!(tree.chunk_generation(VoxelPos::new(5, 5, 5)).unwrap() > g);
    assert_eq!(tree.chunk_generation(VoxelPos::new(9000, 0, 0)), None);
}

#[test]
fn coarse_summaries_keep_the_most_common_material_per_cell() {
    let mut tree = VoxelTree::new();
    // One 16-voxel cell: 3 of stone (1) and 2 of dirt (2), the rest air.
    for (x, m) in [(0, 1), (1, 1), (2, 1), (3, 2), (4, 2)] {
        tree.set(VoxelPos::new(x, 0, 0), MaterialId(m)).unwrap();
    }
    // Another cell, all dirt.
    tree.fill_box(
        world_core::VoxelBox::from_min_size(VoxelPos::new(16, 0, 0), [16; 3]),
        MaterialId(2),
    )
    .unwrap();
    let coarse = tree.extract_coarse(CHUNK_LEVEL, CHUNK, 2).unwrap();
    assert_eq!(coarse.get(VoxelPos::new(15, 15, 15)), MaterialId(1));
    assert_eq!(coarse.get(VoxelPos::new(20, 3, 9)), MaterialId(2));
    assert_eq!(coarse.get(VoxelPos::new(40, 0, 0)), MaterialId::AIR);
    // Whole 16-voxel cells are uniform, so no leaves remain.
    assert_eq!(coarse.stats().leaves, 0);
    assert_eq!(
        representative(TreeRead::root(&tree), world_core::consts::TREE_DEPTH - 1),
        Some(MaterialId(2))
    );
}

#[test]
fn an_unedited_chunk_has_a_one_word_overlay() {
    let base = world_of(&[]);
    let mut world = DenseWorld::new(B);
    world.set(VoxelPos::new(10, 10, 10), MaterialId(3));
    let mut tree = VoxelTree::unloaded();
    tree.build_region(CHUNK_LEVEL, CHUNK, &world).unwrap();
    let overlay = tree.overlay(CHUNK, &world).unwrap();
    assert_eq!(overlay.get(VoxelPos::new(10, 10, 10)), MaterialId::KEEP);
    assert_eq!(overlay.stats().chunks, 0, "uniform KEEP needs no chunk");
    assert!(overlay.overlay(CHUNK, &base).is_ok());
}
