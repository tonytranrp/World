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
        + 3 * b.bricks
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

#[test]
fn chunks_move_between_trees_without_copying() {
    let mut ops_world = DenseWorld::new(B);
    for i in 0..400 {
        ops_world.set(
            VoxelPos::new(8 + i % 70, 8 + i % 30, 8 + (i * 7) % 70),
            MaterialId(1 + (i % 9) as u16),
        );
    }
    let mut worker = VoxelTree::new();
    worker.build_region(CHUNK_LEVEL, CHUNK, &ops_world).unwrap();
    let held = worker.chunks()[0].stats.bytes;
    let data = worker.take_chunk(CHUNK).unwrap();
    assert_eq!(data.bytes(), held, "the chunk's pages travel as they are");
    assert!(!worker.is_resident(CHUNK));
    assert_eq!(worker.stats().chunks, 0);

    let mut world = VoxelTree::unloaded();
    assert_eq!(world.insert_chunk(CHUNK, data.clone()), Ok(true));
    assert!(assert_tree_agrees(&world, &ops_world, B).is_ok());
    check_canonical(&world).unwrap();
    // Edits after the move stay private to the tree that makes them.
    world.set(VoxelPos::new(9, 9, 9), MaterialId(42)).unwrap();
    let mut other = VoxelTree::unloaded();
    other.insert_chunk(CHUNK, data).unwrap();
    assert_ne!(other.get(VoxelPos::new(9, 9, 9)), MaterialId(42));

    // Uniform regions move as a value.
    let mut solid = VoxelTree::new();
    solid
        .fill_box(
            world_core::VoxelBox::from_min_size(CHUNK, [1024; 3]),
            MaterialId(5),
        )
        .unwrap();
    let block = solid.take_chunk(CHUNK).unwrap();
    assert_eq!(block.uniform(), Some(MaterialId(5)));
    let mut target = VoxelTree::unloaded();
    target.insert_chunk(CHUNK, block).unwrap();
    assert_eq!(target.get(VoxelPos::new(500, 500, 500)), MaterialId(5));
    assert_eq!(target.stats().chunks, 0);
}

/// Point reads go straight to a chunk through the chunk index; whatever
/// happens to chunks (moved to another tree and position, unloaded, made
/// uniform, recreated, kept by a snapshot), every read still agrees with a
/// cursor walk from the root, which never uses the index.
#[test]
fn reads_through_the_chunk_index_stay_exact_as_chunks_move_and_go() {
    const C: i32 = 1024;
    let probe = |tree: &world_core::TreeSnapshot, label: &str| {
        let mut cursor = tree.cursor();
        for cx in -1..=1 {
            for cz in -1..=1 {
                for (dx, dy, dz) in [(1, 2, 3), (500, 7, 900), (1023, 0, 1023), (10, 63, 20)] {
                    let p = VoxelPos::new(cx * C + dx, dy, cz * C + dz);
                    assert_eq!(tree.lookup(p), cursor.lookup(p), "{label} at {p:?}");
                }
            }
        }
    };
    let mut tree = VoxelTree::new();
    for cx in -1..=1 {
        for cz in -1..=1 {
            for (dx, dz) in [(1, 3), (500, 900), (10, 20)] {
                let m = MaterialId((cx + 2 + 3 * (cz + 1)) as u16);
                tree.set(VoxelPos::new(cx * C + dx, 2, cz * C + dz), m)
                    .unwrap();
                tree.set(VoxelPos::new(cx * C + dx, 63, cz * C + dz), m)
                    .unwrap();
            }
        }
    }
    let before = tree.snapshot();
    probe(&before, "built");

    // Move a chunk to another tree at another position.
    let data = tree.take_chunk(VoxelPos::new(C, 0, 0)).unwrap();
    probe(&tree.snapshot(), "after take");
    let mut other = VoxelTree::new();
    other.insert_chunk(VoxelPos::new(-C, 0, -C), data).unwrap();
    probe(&other.snapshot(), "moved");
    assert_eq!(
        other.get(VoxelPos::new(-C + 1, 2, -C + 3)),
        before.get(VoxelPos::new(C + 1, 2, 3))
    );

    // Make a chunk uniform, then mixed again; unload one.
    let c = VoxelPos::new(0, 0, 0);
    tree.fill_box(
        world_core::VoxelBox::from_min_size(c, [1024; 3]),
        MaterialId(9),
    )
    .unwrap();
    probe(&tree.snapshot(), "made uniform");
    tree.set(VoxelPos::new(5, 5, 5), MaterialId(1)).unwrap();
    probe(&tree.snapshot(), "mixed again");
    tree.unload(CHUNK_LEVEL, VoxelPos::new(-C, 0, 0)).unwrap();
    let after = tree.snapshot();
    probe(&after, "unloaded");
    assert_eq!(after.lookup(VoxelPos::new(-C + 1, 2, 3)), None);

    // A single voxel write that collapses an indexed chunk to one material,
    // then one that splits it again.
    let corner = VoxelPos::new(0, 0, -C);
    tree.fill_box(
        world_core::VoxelBox::from_min_size(corner, [1024; 3]),
        MaterialId(4),
    )
    .unwrap();
    tree.set(VoxelPos::new(3, 3, -C + 3), MaterialId(5))
        .unwrap();
    probe(&tree.snapshot(), "one odd voxel");
    tree.set(VoxelPos::new(3, 3, -C + 3), MaterialId(4))
        .unwrap();
    let collapsed = tree.snapshot();
    probe(&collapsed, "collapsed by a write");
    assert!(matches!(
        world_core::tree::TreeRead::root(&collapsed),
        world_core::tree::NodeRef::Branch(_)
    ));
    tree.set(VoxelPos::new(700, 9, -C + 1), MaterialId::AIR)
        .unwrap();
    probe(&tree.snapshot(), "split again");
    check_canonical(&tree).unwrap();

    // The snapshot taken first still reads what it saw.
    probe(&before, "old snapshot");
    assert_eq!(before.get(VoxelPos::new(5, 5, 5)), MaterialId::AIR);
}

/// `same_arenas` recognises a chunk's content wherever it moves (into a
/// world tree, from there into another tree, in their snapshots) until
/// something writes to it, and never content merely equal to it.
#[test]
fn same_arenas_follows_a_chunk_until_it_is_written() {
    use world_core::tree::VoxelSource;
    let mut ops_world = DenseWorld::new(B);
    for i in 0..400 {
        ops_world.set(
            VoxelPos::new(8 + i % 70, 8 + i % 30, 8 + (i * 7) % 70),
            MaterialId(1 + (i % 9) as u16),
        );
    }
    let mut worker = VoxelTree::new();
    worker.build_region(CHUNK_LEVEL, CHUNK, &ops_world).unwrap();
    let data = worker.take_chunk(CHUNK).unwrap();

    let mut world = VoxelTree::unloaded();
    world.insert_chunk(CHUNK, data.clone()).unwrap();
    let mut render = VoxelTree::unloaded();
    render
        .insert_chunk(CHUNK, world.chunk_data(CHUNK).unwrap())
        .unwrap();
    let snap = render.snapshot();
    assert!(data.same_arenas(&world.chunk_data(CHUNK).unwrap()));
    assert!(data.same_arenas(&snap.chunk_data(CHUNK).unwrap()));
    assert!(data.same_arenas(&VoxelSource::shared_chunk(&snap, CHUNK).unwrap()));
    assert!(data.same_arenas(&VoxelSource::shared_chunk(&render, CHUNK).unwrap()));

    // A write anywhere copies the arenas: the written tree no longer holds
    // `data`'s content, the others still do.
    world.set(VoxelPos::new(9, 9, 9), MaterialId(42)).unwrap();
    assert!(!data.same_arenas(&world.chunk_data(CHUNK).unwrap()));
    assert!(data.same_arenas(&render.chunk_data(CHUNK).unwrap()));
    render
        .insert_chunk(CHUNK, world.chunk_data(CHUNK).unwrap())
        .unwrap();
    assert!(!data.same_arenas(&render.chunk_data(CHUNK).unwrap()));
    assert!(
        data.same_arenas(&snap.chunk_data(CHUNK).unwrap()),
        "the snapshot is kept"
    );
    // Writing it back to equal content is still another content.
    world
        .set(
            VoxelPos::new(9, 9, 9),
            ops_world.get(VoxelPos::new(9, 9, 9)),
        )
        .unwrap();
    assert!(!data.same_arenas(&world.chunk_data(CHUNK).unwrap()));

    // Equal content built again is not the same arenas.
    let mut again = VoxelTree::new();
    again.build_region(CHUNK_LEVEL, CHUNK, &ops_world).unwrap();
    let rebuilt = again.chunk_data(CHUNK).unwrap();
    assert_eq!(
        again.get(VoxelPos::new(9, 9, 9)),
        snap.get(VoxelPos::new(9, 9, 9))
    );
    assert!(!data.same_arenas(&rebuilt));

    // Air and one material compare by value; nothing where not resident.
    let air = VoxelTree::new().chunk_data(CHUNK).unwrap();
    assert!(air.same_arenas(&VoxelTree::new().snapshot().chunk_data(CHUNK).unwrap()));
    assert!(!air.same_arenas(&data));
    assert!(VoxelTree::unloaded().snapshot().chunk_data(CHUNK).is_none());
    let grid = world_core::DenseGrid::new(3, CHUNK);
    assert!(VoxelSource::shared_chunk(&grid, CHUNK).is_none());
}
