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
        max_summaries_per_frame: 1,
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
        max_summaries_per_frame: budget as u32,
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

/// Nothing in a replaced chunk that waits is expanded or loaded: applying
/// it drops what the table held there.
#[test]
fn a_waiting_chunk_is_not_refined() {
    use world_render::internals::residency::{Change, Refused};
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let min = CHUNKS[1];
    let key = NodeKey::containing(shape.local(min).unwrap(), 4);
    let mut batch = UploadBatch::default();
    res.collapse(key, &mut batch).unwrap();
    tree.insert_chunk(min, replacement(min)).unwrap();
    let snap = tree.snapshot();
    res.queue_changes([Change::Chunk(key)]);
    assert!(res.in_waiting_chunk(key));
    assert!(res.in_waiting_chunk(key.child(0)));
    assert_eq!(res.expand(&snap, key, &mut batch), Err(Refused::Waiting));
    // Applied, it refines as usual.
    res.apply_changes(&snap, None, u64::MAX, |_| true, &mut batch);
    assert!(!res.in_waiting_chunk(key));
    assert!(res.expand(&snap, key, &mut batch).is_ok());
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

/// No frame collapses more than `lod.max_collapses_per_frame` nodes, and a
/// camera that leaves the scene still ends with everything collapsed that
/// a roomy limit collapses.
#[test]
fn a_frame_never_collapses_more_than_its_budget() {
    let tree = world_render::internals::scene::build();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let near = ViewPoint {
        eye: [0.0, 0.0, 0.0],
        pixel_angle: 2.0 * (22.5f32.to_radians()).tan() / 720.0,
    };
    let far = ViewPoint {
        eye: [0.0, 0.0, -20000.0],
        ..near
    };
    let run = |limit: u32| {
        let mut config = BrickCacheConfig::default();
        config.lod.max_collapses_per_frame = limit;
        let (mut res, mut pools) = roomy_cache(shape);
        let mut batch = UploadBatch::default();
        let mut most = 0;
        for frame in 1..=400 {
            let input = FrameInput {
                source: &snap,
                frame,
                refresh: frame == 1,
                dirty: &[],
                chunks: &[],
                view: Some(if frame <= 60 { near } else { far }),
                requests: &[],
                victims: &[],
                config: &config,
                budget: 24 << 20,
                victim_capacity: 64,
            };
            let r = run_frame(&mut res, &input, &mut batch);
            apply_cpu(&mut pools, &batch);
            most = most.max(r.collapsed);
        }
        verify(&mut res, &snap, &pools);
        let s = res.stats();
        (most, s.expanded, s.bricks)
    };
    let roomy = run(1 << 20);
    let tight = run(2);
    assert!(
        roomy.0 > 2,
        "the roomy run collapses more per frame: {roomy:?}"
    );
    assert!(tight.0 <= 2, "{tight:?}");
    assert_eq!((tight.1, tight.2), (roomy.1, roomy.2));
}

/// The shrink pass a node pool pressure change starts walks at the growth
/// budget, never the whole table in one frame, and the pressure holds while
/// it runs.
#[test]
fn a_pressure_change_shrinks_within_the_visit_budget() {
    use world_render::internals::layout::PoolLayout;
    let tree = world_render::internals::scene::build();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let mut config = BrickCacheConfig::default();
    // Small enough that a shrink pass over this scene spans frames.
    config.lod.max_visits_per_frame = 64;
    config.lod.max_shrink_visits_per_frame = 16;
    let view = ViewPoint {
        eye: [0.0, 0.0, 0.0],
        pixel_angle: 2.0 * (22.5f32.to_radians()).tan() / 720.0,
    };
    let layout = PoolLayout {
        node_count: 5 << 9,
        node_groups: 5 << 7,
        ..small_layout()
    };
    let mut res = Residency::new(shape, &layout).expect("node space");
    let mut pools = CpuPools::new(layout.node_count, &layout.segments, layout.brick_ids);
    let mut batch = UploadBatch::default();
    let mut settling_frames = 0;
    for frame in 1..=400 {
        let pressure = res.pressure();
        let settling = res.settling();
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
        run_frame(&mut res, &input, &mut batch);
        apply_cpu(&mut pools, &batch);
        assert!(res.growth().last_shrink_visits() <= 64, "frame {frame}");
        // Held through every frame whose shrink step used its whole budget,
        // so the pass is still running.
        if settling && res.growth().last_shrink_visits() == 64 {
            settling_frames += 1;
            assert_eq!(res.pressure(), pressure, "frame {frame}: held");
        }
    }
    assert!(res.pressure() > 1.0, "the small pool raised the pressure");
    assert!(
        settling_frames > 0,
        "a pressure change started a shrink pass"
    );
    assert!(!res.settling(), "and it finished");
    verify(&mut res, &snap, &pools);
}

/// A rough slab of material 3 over `x0..x0 + 256` and `z0..z0 + 256`, 14
/// voxels thick with holes: 16 mixed level-2 nodes, so summarizing it takes
/// several frames on a small budget.
fn rough_slab(x0: i32, z0: i32) -> VoxelTree {
    let mut other = VoxelTree::new();
    other
        .fill_box(
            VoxelBox::new(
                VoxelPos::new(x0, -16, z0),
                VoxelPos::new(x0 + 256, -2, z0 + 256),
            ),
            MaterialId(3),
        )
        .unwrap();
    for x in (x0..x0 + 256).step_by(5) {
        for z in (z0..z0 + 256).step_by(7) {
            other.set(VoxelPos::new(x, -3, z), MaterialId::AIR).unwrap();
        }
    }
    other
}

/// A small slab of material 3 inside the chunk at `min`.
fn replacement_at(min: VoxelPos) -> world_core::tree::ChunkData {
    let mut other = VoxelTree::new();
    other
        .fill_box(
            VoxelBox::new(
                VoxelPos::new(min.x, -16, min.z),
                VoxelPos::new(min.x + 40, -6, min.z + 40),
            ),
            MaterialId(3),
        )
        .unwrap();
    other.chunk_data(min).expect("mixed content")
}

/// The table is consistent while a replaced chunk waits, even when a
/// sibling's change rewrites the group holding the waiting chunk with a
/// different mask: the waiting chunk's old children are never read through
/// the new mask, and collapsing it frees everything below it.
#[test]
fn a_waiting_chunk_stays_consistent_when_its_parent_group_is_rewritten() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = roomy_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    // c1 holds part of the slab and is expanded; c2, in the same level-5
    // node, is air until it is replaced.
    let c1 = VoxelPos::new(0, -1024, 0);
    let c2 = VoxelPos::new(1024, -1024, 0);
    let key = |min: VoxelPos| NodeKey::containing(shape.local(min).unwrap(), 4);
    assert_eq!(key(c1).parent(), key(c2).parent());
    assert!(res.is_expanded(key(c1)) && res.is_expanded(key(c1).parent()));
    assert!(!res.has_record(key(c2)));
    // c1's new content sits in another level-3 cell, so its mask changes.
    tree.insert_chunk(c1, rough_slab(256, 0).chunk_data(c1).unwrap())
        .unwrap();
    tree.insert_chunk(c2, replacement_at(c2)).unwrap();
    let snap = tree.snapshot();
    let config = BrickCacheConfig {
        max_summaries_per_frame: 1,
        ..Default::default()
    };
    // Over c2, with a pixel angle so fine that stage 1 keeps everything.
    let view = ViewPoint {
        eye: [c2.x as f32 + 40.0, -10.0, c2.z as f32 + 40.0],
        pixel_angle: 1e-6,
    };
    let both = [c1, c2];
    let mut frame = 10;
    while !res.has_record(key(c2)) {
        step(
            &mut res,
            &mut pools,
            frame,
            Frame {
                source: &snap,
                dirty: &[],
                chunks: if frame == 10 { &both } else { &[] },
                view: Some(view),
                config: &config,
            },
        );
        frame += 1;
        assert!(frame < 30, "c2 was never applied");
    }
    assert!(res.in_waiting_chunk(key(c1)), "c1 still waits");
    verify(&mut res, &snap, &pools);

    // Collapsing the waiting chunk leaks nothing: after a reset no node
    // group and no brick id is left in use.
    let mut batch = UploadBatch::default();
    let _ = res.collapse(key(c1), &mut batch);
    apply_cpu(&mut pools, &batch);
    verify(&mut res, &snap, &pools);
    let mut batch = UploadBatch::default();
    res.reset(&snap, &mut batch);
    let s = res.stats();
    assert_eq!((s.expanded, s.bricks), (0, 0), "{s:?}");
}

/// An edit inside a replaced chunk that waits is folded into the chunk's
/// own change, which rebuilds it anyway: it never uploads new content into
/// the old structure or summarizes the new chunk outside the budget.
#[test]
fn an_edit_inside_a_waiting_chunk_waits_for_the_chunk() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = roomy_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let c1 = VoxelPos::new(0, -1024, 0);
    tree.insert_chunk(c1, rough_slab(256, 0).chunk_data(c1).unwrap())
        .unwrap();
    // An edit in a brick that is resident in the old content.
    let at = VoxelPos::new(3, -5, 3);
    let dirty = tree
        .apply(&[world_core::Edit::new(at, MaterialId(2))])
        .unwrap();
    let snap = tree.snapshot();
    let budget = 4u64;
    let config = BrickCacheConfig {
        max_summaries_per_frame: budget as u32,
        ..Default::default()
    };
    let view = ViewPoint {
        eye: [at.x as f32, at.y as f32, at.z as f32],
        pixel_angle: 1e-6,
    };
    let one = [c1];
    let mut frame = 10;
    loop {
        let before = res.summaries_computed();
        let r = step(
            &mut res,
            &mut pools,
            frame,
            Frame {
                source: &snap,
                dirty: if frame == 10 { dirty.as_slice() } else { &[] },
                chunks: if frame == 10 { &one } else { &[] },
                view: Some(view),
                config: &config,
            },
        );
        let spent = res.summaries_computed() - before;
        // The budget, one grandchild (65), the chunk's path once applied.
        let slack = 65 + u64::from(shape.root_level()) + 64;
        assert!(spent <= budget + slack, "frame {frame}: {spent} summaries");
        frame += 1;
        if r.changes_pending == 0 {
            break;
        }
        assert!(frame < 200, "never applied");
    }
    verify(&mut res, &snap, &pools);
}

/// Changed bricks count against the summary budget too: many edits on a
/// budget of one apply about one a frame, nearest first, and converge.
#[test]
fn many_edits_spread_over_frames_on_a_small_budget() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = roomy_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let edits: Vec<world_core::Edit> = (0..12)
        .map(|i| world_core::Edit::new(VoxelPos::new(-60 + 10 * i, -5, 3), MaterialId(2)))
        .collect();
    let dirty = tree.apply(&edits).unwrap();
    assert!(dirty.len() >= 8, "{} bricks", dirty.len());
    let snap = tree.snapshot();
    let config = BrickCacheConfig {
        max_summaries_per_frame: 1,
        ..Default::default()
    };
    // At the far end of the row: the nearest brick goes first.
    let view = ViewPoint {
        eye: [60.0, -5.0, 3.0],
        pixel_angle: 1e-6,
    };
    let nearest = shape
        .brick_key(world_core::BrickPos::containing(VoxelPos::new(50, -5, 3)))
        .unwrap();
    let edited_material =
        |res: &Residency| res.record(nearest).map(|r| r.edited()).unwrap_or_default();
    let mut frame = 10;
    loop {
        let r = step(
            &mut res,
            &mut pools,
            frame,
            Frame {
                source: &snap,
                dirty: if frame == 10 { dirty.as_slice() } else { &[] },
                chunks: &[],
                view: Some(view),
                config: &config,
            },
        );
        if frame == 10 {
            assert_eq!(r.changes_applied, 1, "{r:?}");
            assert!(edited_material(&res), "the nearest brick went first");
        }
        frame += 1;
        if r.changes_pending == 0 {
            break;
        }
        assert!(frame < 100, "never drained");
    }
    assert!(frame - 10 >= dirty.len() as u32, "one a frame");
    verify(&mut res, &snap, &pools);
}

/// A change that waits below an expanded node changes that node's mask in
/// the source; when a change elsewhere rewrites the group above it, the
/// node keeps its record and everything resident below it, instead of
/// dropping its whole subtree for stage 1 to rebuild (which, walking, threw
/// away up to 600,000 records in one frame).
#[test]
fn a_waiting_change_below_keeps_its_ancestors_resident() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = roomy_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let key = |min: VoxelPos| NodeKey::containing(shape.local(min).unwrap(), 4);
    // c0 holds part of the slab; c1, in the same level-5 node, is air until
    // replaced and then waits; c3, in another level-5 node of the same
    // level-6 node, is air until replaced and is applied first.
    let c0 = VoxelPos::new(0, -1024, 0);
    let c1 = VoxelPos::new(1024, -1024, 0);
    let c3 = VoxelPos::new(4096, -1024, 0);
    assert_eq!(key(c0).parent(), key(c1).parent());
    assert_ne!(key(c0).parent(), key(c3).parent());
    assert_eq!(key(c0).ancestor(6), key(c3).ancestor(6));
    assert!(res.is_expanded(key(c0)));
    let expanded_before = res.stats().expanded;
    tree.insert_chunk(c1, rough_slab(1024, 0).chunk_data(c1).unwrap())
        .unwrap();
    tree.insert_chunk(c3, replacement_at(c3)).unwrap();
    let snap = tree.snapshot();
    let config = BrickCacheConfig {
        max_summaries_per_frame: 1,
        ..Default::default()
    };
    let view = ViewPoint {
        eye: [c3.x as f32 + 40.0, -10.0, c3.z as f32 + 40.0],
        pixel_angle: 1e-6,
    };
    let both = [c1, c3];
    let mut frame = 10;
    while !res.has_record(key(c3)) {
        let r = step(
            &mut res,
            &mut pools,
            frame,
            Frame {
                source: &snap,
                dirty: &[],
                chunks: if frame == 10 { &both } else { &[] },
                view: Some(view),
                config: &config,
            },
        );
        assert_eq!(r.collapsed, 0);
        frame += 1;
        assert!(frame < 30, "c3 was never applied");
    }
    assert!(res.in_waiting_chunk(key(c1)), "c1 still waits");
    assert!(res.is_expanded(key(c0)), "c0 kept its detail");
    assert!(
        res.stats().expanded >= expanded_before,
        "{} expanded, {expanded_before} before",
        res.stats().expanded
    );
    verify(&mut res, &snap, &pools);
}
