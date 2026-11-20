//! CPU-only checks of the brick cache's per-frame budgets (decisions 0010
//! and 0012): replaced chunks are rebuilt from their new content, changes
//! beyond the summary budget wait and are applied nearest first, an edit
//! costs summaries in proportion to what changed, expansions per frame are
//! bounded, and a growth pass is bounded in pops and pushes and survives
//! small camera moves.

mod common;

use common::{apply_cpu, cpu_cache, make_resident, small_layout};
use world_core::tree::VoxelSource;
use world_core::{BrickPos, MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_render::internals::BrickCacheConfig;
use world_render::internals::batch::{CpuPools, UploadBatch};
use world_render::internals::frame::{FrameInput, FrameReport, run_frame};
use world_render::internals::key::{NodeKey, Shape, view_at};
use world_render::internals::residency::Residency;
use world_render::internals::select::{GrowthState, ViewPoint, select_budgeted};

/// A ground slab across four chunks, its bricks mixed so they need blocks.
fn slab() -> VoxelTree {
    let mut tree = VoxelTree::new();
    tree.fill_box(
        VoxelBox::new(VoxelPos::new(-64, -16, -64), VoxelPos::new(64, 0, 64)),
        MaterialId(1),
    )
    .unwrap();
    for x in (-64..64).step_by(8) {
        for z in (-64..64).step_by(8) {
            tree.set(VoxelPos::new(x, -1, z), MaterialId::AIR).unwrap();
        }
    }
    tree
}

/// Different content for the chunk at `min`: a thinner slab of another
/// material in one quarter of the old one's footprint, with holes.
fn replacement(min: VoxelPos) -> world_core::tree::ChunkData {
    let mut other = VoxelTree::new();
    let x0 = if min.x < 0 { -40 } else { 0 };
    let z0 = if min.z < 0 { -40 } else { 0 };
    other
        .fill_box(
            VoxelBox::new(
                VoxelPos::new(x0, -16, z0),
                VoxelPos::new(x0 + 40, -6, z0 + 40),
            ),
            MaterialId(3),
        )
        .unwrap();
    for x in (x0..x0 + 40).step_by(4) {
        other
            .set(VoxelPos::new(x, -7, z0 + 5), MaterialId(4))
            .unwrap();
        other
            .set(VoxelPos::new(x, -9, z0 + 9), MaterialId::AIR)
            .unwrap();
    }
    other.chunk_data(min).expect("mixed content")
}

/// The four chunks the slab spans.
const CHUNKS: [VoxelPos; 4] = [
    VoxelPos::new(-1024, -1024, -1024),
    VoxelPos::new(0, -1024, -1024),
    VoxelPos::new(-1024, -1024, 0),
    VoxelPos::new(0, -1024, 0),
];

/// A table and pools with room for every brick of the debug scene, so the
/// order of loads cannot change what fits.
fn roomy_cache(shape: Shape) -> (Residency, CpuPools) {
    let layout = world_render::internals::layout::PoolLayout {
        segments: vec![8 << 20, 8 << 20],
        brick_ids: 1 << 16,
        allocs_per_segment: 1 << 16,
        ..small_layout()
    };
    let res = Residency::new(shape, &layout).expect("node space");
    let pools = CpuPools::new(layout.node_count, &layout.segments, layout.brick_ids);
    (res, pools)
}

fn verify(res: &mut Residency, source: &impl VoxelSource, pools: &CpuPools) {
    let segs: Vec<&[u32]> = pools.segments.iter().map(Vec::as_slice).collect();
    assert_eq!(
        res.verify(source, &pools.nodes, &segs, &pools.table),
        Ok(())
    );
}

struct Frame<'a, S: VoxelSource> {
    source: &'a S,
    dirty: &'a [BrickPos],
    chunks: &'a [VoxelPos],
    view: Option<ViewPoint>,
    config: &'a BrickCacheConfig,
}

fn step<S: VoxelSource + Sync>(
    res: &mut Residency,
    pools: &mut CpuPools,
    frame: u32,
    f: Frame<'_, S>,
) -> FrameReport {
    let mut batch = UploadBatch::default();
    let input = FrameInput {
        source: f.source,
        frame,
        refresh: false,
        dirty: f.dirty,
        chunks: f.chunks,
        view: f.view,
        requests: &[],
        victims: &[],
        config: f.config,
        budget: 64 << 20,
        victim_capacity: 64,
    };
    let report = run_frame(res, &input, &mut batch);
    apply_cpu(pools, &batch);
    report
}

/// A chunk replaced wholesale (a load, an unload, a swap between full and
/// coarse detail) is rebuilt from its new content, including everything
/// that was resident below it, not just the path to one of its bricks.
#[test]
fn a_replaced_chunk_is_rebuilt_from_its_new_content() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let config = BrickCacheConfig::default();

    let min = CHUNKS[3];
    tree.insert_chunk(min, replacement(min)).unwrap();
    let snap = tree.snapshot();
    let r = step(
        &mut res,
        &mut pools,
        10,
        Frame {
            source: &snap,
            dirty: &[],
            chunks: &[min],
            view: None,
            config: &config,
        },
    );
    assert_eq!((r.changes_applied, r.changes_pending), (1, 0));
    verify(&mut res, &snap, &pools);

    // Unloaded: the region becomes not resident.
    tree.take_chunk(min).unwrap();
    let snap = tree.snapshot();
    step(
        &mut res,
        &mut pools,
        11,
        Frame {
            source: &snap,
            dirty: &[],
            chunks: &[min],
            view: None,
            config: &config,
        },
    );
    verify(&mut res, &snap, &pools);
}

/// Changes whose summaries exceed the per-frame budget wait: each frame
/// makes progress on the nearest, and once all are applied the table
/// matches the source exactly.
#[test]
fn changes_over_the_summary_budget_wait_and_go_nearest_first() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let config = BrickCacheConfig {
        max_change_summaries_per_frame: 1,
        ..Default::default()
    };
    for min in CHUNKS {
        tree.insert_chunk(min, replacement(min)).unwrap();
    }
    let snap = tree.snapshot();
    let chunk_key = |min: VoxelPos| NodeKey::containing(shape.local(min).unwrap(), 4);
    // The new content is mostly material 3, the old one material 1.
    let chunk_material = |_: VoxelPos| MaterialId(3);
    let mut applied_order = Vec::new();
    let mut frame = 10;
    loop {
        let before: Vec<bool> = CHUNKS
            .iter()
            .map(|&m| res.record(chunk_key(m)).map(|r| r.material()) == Some(chunk_material(m)))
            .collect();
        let r = step(
            &mut res,
            &mut pools,
            frame,
            Frame {
                source: &snap,
                dirty: &[],
                chunks: if frame == 10 { &CHUNKS } else { &[] },
                view: None,
                config: &config,
            },
        );
        assert!(r.changes_applied <= 1, "{r:?}");
        for (i, &m) in CHUNKS.iter().enumerate() {
            let now = res.record(chunk_key(m)).map(|r| r.material()) == Some(chunk_material(m));
            if now && !before[i] {
                applied_order.push(i);
            }
        }
        frame += 1;
        if r.changes_pending == 0 {
            break;
        }
        assert!(frame < 40, "the changes never drained");
    }
    assert!(frame > 14, "the budget spread the changes over frames");
    applied_order.sort_unstable();
    assert_eq!(applied_order, [0, 1, 2, 3], "each applied once");
    verify(&mut res, &snap, &pools);

    // With a view, the nearest change goes first.
    let mut tree = slab();
    let snap = tree.snapshot();
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    for min in CHUNKS {
        tree.insert_chunk(min, replacement(min)).unwrap();
    }
    let snap = tree.snapshot();
    // Over the last chunk, with a pixel angle so fine that stage 1 keeps
    // everything resident.
    let near = CHUNKS[3];
    let view = ViewPoint {
        eye: [near.x as f32 + 40.0, 0.0, near.z as f32 + 40.0],
        pixel_angle: 1e-6,
    };
    let mut frame = 10;
    loop {
        let r = step(
            &mut res,
            &mut pools,
            frame,
            Frame {
                source: &snap,
                dirty: &[],
                chunks: if frame == 10 { &CHUNKS } else { &[] },
                view: Some(view),
                config: &config,
            },
        );
        frame += 1;
        if r.changes_applied > 0 {
            assert_eq!((r.changes_applied, r.changes_pending), (1, 3), "{r:?}");
            break;
        }
        assert!(frame < 20, "nothing was applied");
    }
    let material = |min: VoxelPos| res.record(chunk_key(min)).map(|r| r.material());
    assert_eq!(
        material(near),
        Some(MaterialId(3)),
        "the nearest chunk went first"
    );
    for &m in &CHUNKS[..3] {
        assert_eq!(material(m), Some(MaterialId(1)), "{m:?} waits");
    }
}

/// A replaced chunk too large for one frame's budget is summarized over
/// several frames, each within the budget plus one grandchild's summaries
/// (at most 65) and the chunk's own path once it is applied.
#[test]
fn a_large_replaced_chunk_is_summarized_across_frames() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let budget = 20u64;
    let config = BrickCacheConfig {
        max_change_summaries_per_frame: budget as u32,
        ..Default::default()
    };
    // A rough slab over a 256 x 256 footprint: 16 mixed level-2 nodes of
    // mixed bricks.
    let min = CHUNKS[3];
    let mut other = VoxelTree::new();
    other
        .fill_box(
            VoxelBox::new(VoxelPos::new(0, -16, 0), VoxelPos::new(256, -2, 256)),
            MaterialId(3),
        )
        .unwrap();
    for x in (0..256).step_by(5) {
        for z in (0..256).step_by(7) {
            other.set(VoxelPos::new(x, -3, z), MaterialId::AIR).unwrap();
        }
    }
    tree.insert_chunk(min, other.chunk_data(min).unwrap())
        .unwrap();
    let snap = tree.snapshot();
    let key = NodeKey::containing(shape.local(min).unwrap(), 4);
    let mixed = count_mixed(view_at(&snap, key), 4);
    assert!(mixed > 3 * budget, "{mixed} mixed nodes");
    let mins = [min];
    let mut frame = 10;
    loop {
        let before = res.summaries_computed();
        let r = step(
            &mut res,
            &mut pools,
            frame,
            Frame {
                source: &snap,
                dirty: &[],
                chunks: if frame == 10 { &mins } else { &[] },
                view: None,
                config: &config,
            },
        );
        let spent = res.summaries_computed() - before;
        let slack = 65 + u64::from(shape.root_level()) + 64;
        assert!(spent <= budget + slack, "frame {frame}: {spent} summaries");
        frame += 1;
        if r.changes_pending == 0 {
            break;
        }
        assert_eq!(r.changes_applied, 0, "applied only once summarized");
        assert!(frame < 200, "never applied");
    }
    assert!(frame > 13, "spread over frames");
    verify(&mut res, &snap, &pools);
}

/// An edit recomputes one summary per level on its path, however much of
/// the world is resident; a replaced chunk about one per mixed node in it.
#[test]
fn an_edit_costs_summaries_in_proportion_to_what_changed() {
    let config = BrickCacheConfig::default();
    let edit_cost = |tree: &mut VoxelTree| {
        let snap = tree.snapshot();
        let shape = Shape::of(&snap);
        let (mut res, mut pools) = cpu_cache(shape);
        make_resident(&mut res, &snap, &mut pools);
        let dirty = tree
            .apply(&[world_core::Edit::new(
                VoxelPos::new(3, -5, 3),
                MaterialId(2),
            )])
            .unwrap();
        let snap = tree.snapshot();
        let before = res.summaries_computed();
        step(
            &mut res,
            &mut pools,
            10,
            Frame {
                source: &snap,
                dirty: dirty.as_slice(),
                chunks: &[],
                view: None,
                config: &config,
            },
        );
        verify(&mut res, &snap, &pools);
        (res.summaries_computed() - before, shape.root_level())
    };
    let (small, levels) = edit_cost(&mut slab());
    let mut big = slab();
    big.fill_box(
        VoxelBox::new(VoxelPos::new(-64, -48, -64), VoxelPos::new(64, -32, 64)),
        MaterialId(5),
    )
    .unwrap();
    for x in (-64..64).step_by(8) {
        big.set(VoxelPos::new(x, -40, 7), MaterialId::AIR).unwrap();
    }
    let (large, _) = edit_cost(&mut big);
    assert!(small <= u64::from(levels), "{small} summaries for one edit");
    assert_eq!(small, large, "the cost does not grow with the world");

    // A replaced chunk: its mixed nodes, plus the path above it.
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let min = CHUNKS[0];
    tree.insert_chunk(min, replacement(min)).unwrap();
    let snap = tree.snapshot();
    let key = NodeKey::containing(shape.local(min).unwrap(), 4);
    let mixed = count_mixed(view_at(&snap, key), 4);
    let before = res.summaries_computed();
    step(
        &mut res,
        &mut pools,
        10,
        Frame {
            source: &snap,
            dirty: &[],
            chunks: &[min],
            view: None,
            config: &config,
        },
    );
    let cost = res.summaries_computed() - before;
    assert!(
        cost <= mixed + u64::from(shape.root_level()),
        "{cost} summaries for a chunk of {mixed} mixed nodes"
    );
    verify(&mut res, &snap, &pools);
}

/// Mixed nodes of level 1 and up under and including `node` (of `level`).
fn count_mixed<N: world_core::tree::NodeView>(node: N, level: u32) -> u64 {
    use world_core::tree::NodeKind;
    if level == 0 || node.kind() != NodeKind::Branch {
        return 0;
    }
    1 + node
        .children()
        .map(|(_, c)| count_mixed(c, level - 1))
        .sum::<u64>()
}

/// No frame expands more nodes than `lod.max_expansions_per_frame`, and a
/// tight limit reaches the same cache as a roomy one, later.
#[test]
fn a_frame_never_expands_more_than_its_budget() {
    let tree = world_render::internals::scene::build();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let view = ViewPoint {
        eye: [0.0, 0.0, 0.0],
        pixel_angle: 2.0 * (22.5f32.to_radians()).tan() / 720.0,
    };
    let run = |limit: u32, frames: u32| {
        let mut config = BrickCacheConfig::default();
        config.lod.max_expansions_per_frame = limit;
        let (mut res, mut pools) = roomy_cache(shape);
        let mut batch = UploadBatch::default();
        let mut most = 0;
        let mut late = 0;
        for frame in 1..=frames {
            let input = FrameInput {
                source: &snap,
                frame,
                refresh: frame == 1,
                dirty: &[],
                chunks: &[],
                view: Some(view),
                requests: &[],
                victims: &[],
                config: &config,
                budget: 24 << 20,
                victim_capacity: 64,
            };
            let r = run_frame(&mut res, &input, &mut batch);
            apply_cpu(&mut pools, &batch);
            most = most.max(r.expanded);
            if frame > frames - 5 {
                late += r.expanded + r.loaded;
            }
        }
        verify(&mut res, &snap, &pools);
        let s = res.stats();
        (most, late, s.expanded, s.bricks)
    };
    let roomy = run(1 << 20, 60);
    let tight = run(3, 600);
    assert!(
        roomy.0 > 3,
        "the roomy run expands more per frame: {roomy:?}"
    );
    assert!(tight.0 <= 3, "{tight:?}");
    assert_eq!((roomy.1, tight.1), (0, 0), "both settled");
    assert_eq!((tight.2, tight.3), (roomy.2, roomy.3));
}

/// A growth step makes at most `max_visits` queue pops and pushes, plus
/// the pushes of the one node that crossed the limit.
#[test]
fn a_growth_step_is_bounded_in_pops_and_pushes() {
    let tree = world_render::internals::scene::build();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let lod = BrickCacheConfig::default().lod;
    let view = ViewPoint {
        eye: [0.0, 0.0, 0.0],
        pixel_angle: 1e-4,
    };
    // Collapse everything below the forced levels, so a pass has work.
    let mut batch = UploadBatch::default();
    res.reset(&snap, &mut batch);
    let mut walk = Vec::new();
    let mut growth = GrowthState::default();
    for budget in [1u32, 2, 16, 100] {
        let mut steps = 0;
        loop {
            select_budgeted(&res, &mut walk, &mut growth, &view, &lod, 1.0, budget);
            assert!(
                growth.last_visits() < budget + 64,
                "{} visits on a budget of {budget}",
                growth.last_visits()
            );
            steps += 1;
            if !growth.is_active() {
                break;
            }
        }
        assert!(steps >= 1);
        growth = GrowthState::default();
    }
}

/// A growth pass in progress keeps going when the camera moves less than
/// `lod.restart_voxels`, since every node it takes is judged from the
/// current view; a larger move starts it over from the new view.
#[test]
fn a_growth_pass_survives_small_moves() {
    let tree = world_render::internals::scene::build();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, _pools) = cpu_cache(shape);
    let config = BrickCacheConfig::default();
    let lod = config.lod;
    assert!(lod.restart_voxels > 2.0 * lod.regrow_voxels);
    let view = ViewPoint {
        eye: [0.0, 0.0, 0.0],
        pixel_angle: 2.0 * (22.5f32.to_radians()).tan() / 720.0,
    };
    // A few frames on a small budget: partly refined, much left to do.
    let mut batch = UploadBatch::default();
    for frame in 1..=8 {
        let input = FrameInput {
            source: &snap,
            frame,
            refresh: frame == 1,
            dirty: &[],
            chunks: &[],
            view: Some(view),
            requests: &[],
            victims: &[],
            config: &config,
            budget: 256 << 10,
            victim_capacity: 64,
        };
        run_frame(&mut res, &input, &mut batch);
    }
    let mut walk = Vec::new();
    let mut growth = GrowthState::default();
    select_budgeted(&res, &mut walk, &mut growth, &view, &lod, 1.0, 2);
    assert!(growth.is_active());
    assert_eq!(growth.started_at(), Some(view));

    let small = ViewPoint {
        eye: [0.0, 0.0, 2.0 * lod.regrow_voxels],
        ..view
    };
    select_budgeted(&res, &mut walk, &mut growth, &small, &lod, 1.0, 2);
    assert_eq!(
        growth.started_at(),
        Some(view),
        "a small move keeps the pass"
    );

    let large = ViewPoint {
        eye: [0.0, 0.0, 2.0 * lod.restart_voxels],
        ..view
    };
    select_budgeted(&res, &mut walk, &mut growth, &large, &lod, 1.0, 2);
    assert_eq!(growth.started_at(), Some(large), "a large move restarts it");
}
