//! CPU-only checks of the GPU format, the residency table and streaming:
//! the encoder round trip through the CPU decoder, consistency of the table
//! and the pools under random edits, requests and evictions, budget order,
//! stage-1 hysteresis, the shader constant mirror and the shipped config.

mod common;

use common::{apply_cpu, cpu_cache, make_resident};
use proptest::prelude::*;
use world_core::tree::{NodeKind, NodeView, VoxelSource};
use world_core::{DenseGrid, MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_render::internals::batch::{CpuPools, UploadBatch};
use world_render::internals::feedback::{Request, encode_request};
use world_render::internals::format::decode::Sample;
use world_render::internals::frame::{FrameInput, run_frame};
use world_render::internals::key::{NodeKey, Shape, view_at};
use world_render::internals::select::{ViewPoint, projected_pixels, select};
use world_render::internals::{BrickCacheConfig, LodTuning, Stage};
use world_testkit::dense::{Bounds, EditOp, VoxelModel};
use world_testkit::profile::scale;
use world_testkit::strategies::edit_ops;

/// What the pools say about one voxel, checked against the source: exact
/// voxels must equal it, air must be air, a coarse cell must contain at
/// least one solid voxel of the current source.
fn check_voxel<S: VoxelSource>(
    source: &S,
    pools: &CpuPools,
    shape: Shape,
    pos: VoxelPos,
) -> Result<(), String> {
    let local = shape.local(pos).ok_or("outside")?;
    let sample = pools
        .with_view(|v| v.sample(shape.root_level(), local))
        .map_err(|e| format!("{pos:?}: {e}"))?;
    let truth = source.get(pos);
    match sample {
        Sample::Exact(m) if m == truth => Ok(()),
        Sample::Air if truth.is_air() => Ok(()),
        Sample::Coarse { level, .. } => {
            // The cell is child `slot` of the level-`level` node holding the voxel.
            let node = view_at(source, NodeKey::containing(local, level));
            let cell = node.child(world_core::coords::path::child_slot(local, level));
            if cell.kind() == NodeKind::Air {
                Err(format!(
                    "{pos:?}: coarse cell of level {level} has no solid voxel"
                ))
            } else {
                Ok(())
            }
        }
        other => Err(format!(
            "{pos:?}: pools say {other:?}, source says {truth:?}"
        )),
    }
}

fn check_box<S: VoxelSource>(
    source: &S,
    pools: &CpuPools,
    shape: Shape,
    region: VoxelBox,
) -> Result<(), String> {
    let (lo, end) = (region.min(), region.end());
    for z in lo.z..end.z {
        for y in lo.y..end.y {
            for x in lo.x..end.x {
                check_voxel(source, pools, shape, VoxelPos::new(x, y, z))?;
            }
        }
    }
    Ok(())
}

/// Every brick a box touches (`fill_box` reports no bricks).
fn bricks_of(region: VoxelBox) -> Vec<world_core::BrickPos> {
    let mut out = Vec::new();
    if region.is_empty() {
        return out;
    }
    let (lo, end) = (region.min(), region.end());
    let bl = world_core::BrickPos::containing(lo);
    let bh = world_core::BrickPos::containing(VoxelPos::new(end.x - 1, end.y - 1, end.z - 1));
    for z in bl.z..=bh.z {
        for y in bl.y..=bh.y {
            for x in bl.x..=bh.x {
                out.push(world_core::BrickPos::new(x, y, z));
            }
        }
    }
    out
}

fn grid_with(ops: &[EditOp]) -> DenseGrid {
    let mut g = DenseGrid::new(3, VoxelPos::new(-32, 0, 16));
    for op in ops {
        match *op {
            EditOp::Set(e) => {
                g.set(e.pos, e.material);
            }
            EditOp::Fill { region, material } => {
                let (lo, end) = (region.min(), region.end());
                for z in lo.z..end.z {
                    for y in lo.y..end.y {
                        for x in lo.x..end.x {
                            g.set(VoxelPos::new(x, y, z), material);
                        }
                    }
                }
            }
        }
    }
    g
}

const TREE_BOUNDS: Bounds = Bounds::new(VoxelPos::new(-20, -9, -40), [40, 24, 36]);

fn cases(divisor: u32) -> ProptestConfig {
    ProptestConfig::with_cases((scale().cases / divisor).max(8))
}

proptest! {
    #![proptest_config(cases(4))]

    /// Every voxel of a random dense grid reads back through the GPU format
    /// exactly, with every node expanded and every brick resident.
    #[test]
    fn grids_round_trip_through_the_gpu_format(
        ops in edit_ops(Bounds::new(VoxelPos::new(-32, 0, 16), [64, 64, 64]), 6, 24)
    ) {
        let grid = grid_with(&ops);
        let shape = Shape::of(&grid);
        let (mut res, mut pools) = cpu_cache(shape);
        make_resident(&mut res, &grid, &mut pools);
        let region = grid.bounds();
        prop_assert_eq!(check_box(&grid, &pools, shape, region), Ok(()));
        // Fully resident: nothing may be coarse.
        let (lo, end) = (region.min(), region.end());
        for z in (lo.z..end.z).step_by(3) {
            for y in (lo.y..end.y).step_by(3) {
                for x in lo.x..end.x {
                    let pos = VoxelPos::new(x, y, z);
                    let s = pools.with_view(|v| v.sample(shape.root_level(), shape.local(pos).unwrap()));
                    prop_assert!(!matches!(s, Ok(Sample::Coarse { .. })), "{pos:?} coarse");
                }
            }
        }
        let segs: Vec<&[u32]> = pools.segments.iter().map(Vec::as_slice).collect();
        prop_assert_eq!(res.verify(&grid, &pools.nodes, &segs, &pools.table), Ok(()));
    }

    /// The same for the world tree, with random edits near the origin.
    #[test]
    fn world_trees_round_trip_through_the_gpu_format(ops in edit_ops(TREE_BOUNDS, 8, 32)) {
        let mut tree = VoxelTree::new();
        for op in &ops {
            tree.apply_op(*op);
        }
        let snap = tree.snapshot();
        let shape = Shape::of(&snap);
        let (mut res, mut pools) = cpu_cache(shape);
        make_resident(&mut res, &snap, &mut pools);
        let b = TREE_BOUNDS.as_box();
        let grown = VoxelBox::new(
            VoxelPos::new(b.min().x - 2, b.min().y - 2, b.min().z - 2),
            VoxelPos::new(b.end().x + 2, b.end().y + 2, b.end().z + 2),
        );
        prop_assert_eq!(check_box(&snap, &pools, shape, grown), Ok(()));
    }

    /// Random edits, collapses, evictions, loads and expansions keep the
    /// table equal to the pools and the pools never stale: after every step
    /// each voxel is exact, air or inside a coarse cell that is solid now.
    #[test]
    fn residency_stays_consistent_under_random_operations(
        steps in prop::collection::vec((edit_ops(TREE_BOUNDS, 6, 6), 0u8..6, any::<u16>(), any::<bool>()), 1..10)
    ) {
        let mut tree = VoxelTree::new();
        tree.fill_box(VoxelBox::new(VoxelPos::new(-20, -9, -40), VoxelPos::new(20, -2, -4)), MaterialId(1)).unwrap();
        let shape = Shape::of(&tree);
        let (mut res, mut pools) = cpu_cache(shape);
        make_resident(&mut res, &tree.snapshot(), &mut pools);
        let mut batch = UploadBatch::default();
        for (frame, (ops, action, pick, upload_edits)) in steps.into_iter().enumerate() {
            res.set_frame(frame as u32 + 2);
            let mut dirty = Vec::new();
            for op in ops {
                match op {
                    EditOp::Set(e) => {
                        let d = tree.apply(&[e]).unwrap();
                        dirty.extend_from_slice(d.as_slice());
                    }
                    EditOp::Fill { region, material } => {
                        tree.fill_box(region, material).unwrap();
                        dirty.extend(bricks_of(region));
                    }
                }
            }
            let snap = tree.snapshot();
            let keys: Vec<NodeKey> = dirty.iter().filter_map(|&b| shape.brick_key(b)).collect();
            res.apply_edits(&snap, &keys, |_| upload_edits, &mut batch);
            let all = res.keys();
            if !all.is_empty() {
                let k = all[pick as usize % all.len()];
                let _ = match action {
                    0 => res.collapse(k, &mut batch),
                    1 => res.evict_brick(k, &mut batch),
                    2 => res.load_brick(&snap, k, &mut batch).map(|_| ()),
                    3 => res.expand(&snap, k, &mut batch).map(|_| ()),
                    _ => Ok(()),
                };
            }
            apply_cpu(&mut pools, &batch);
            batch.clear();
            let segs: Vec<&[u32]> = pools.segments.iter().map(Vec::as_slice).collect();
            prop_assert_eq!(res.verify(&snap, &pools.nodes, &segs, &pools.table), Ok(()));
            prop_assert_eq!(check_box(&snap, &pools, shape, TREE_BOUNDS.as_box()), Ok(()));
        }
    }
}

/// A ground slab of resident bricks around the origin.
fn slab() -> VoxelTree {
    let mut tree = VoxelTree::new();
    tree.fill_box(
        VoxelBox::new(VoxelPos::new(-64, -16, -64), VoxelPos::new(64, 0, 64)),
        MaterialId(1),
    )
    .unwrap();
    // Make bricks non-uniform so they need blocks.
    for x in (-64..64).step_by(8) {
        for z in (-64..64).step_by(8) {
            tree.set(VoxelPos::new(x, -1, z), MaterialId::AIR).unwrap();
        }
    }
    tree
}

#[test]
fn edited_bricks_go_first_and_get_at_least_a_quarter_of_the_budget() {
    let mut tree = slab();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let resident = res.resident_bricks();
    assert!(resident.len() >= 32);

    // Evict half the bricks so there are loads to compete with edits.
    let mut batch = UploadBatch::default();
    for k in resident.iter().skip(1).step_by(2) {
        res.evict_brick(*k, &mut batch).unwrap();
    }
    apply_cpu(&mut pools, &batch);

    // Edit every still-resident brick.
    let mut edits = Vec::new();
    for k in resident.iter().step_by(2) {
        let p = k.brick_pos(shape.origin).min_voxel();
        edits.push(world_core::Edit::new(
            VoxelPos::new(p.x + 3, p.y + 3, p.z + 3),
            MaterialId(2),
        ));
    }
    let dirty = tree.apply(&edits).unwrap();
    let snap = tree.snapshot();
    let requests: Vec<Request> = resident
        .iter()
        .skip(1)
        .step_by(2)
        .map(|&key| Request { key, priority: 100 })
        .collect();
    let config = BrickCacheConfig {
        stage: Stage::RayGuided,
        ..Default::default()
    };
    // The block size of one edited brick, by encoding it.
    let brick_bytes = {
        let view = world_render::internals::key::view_at(&snap, resident[0]);
        let mut words = Vec::new();
        let e = world_render::internals::format::brick::encode_brick(
            view,
            0,
            true,
            &mut words,
            &mut Default::default(),
        );
        u64::from(e.words) * 4
    };
    // Room for about eight bricks.
    let budget = 8 * brick_bytes + 512;
    let input = FrameInput {
        source: &snap,
        frame: 10,
        refresh: false,
        dirty: dirty.as_slice(),
        view: None,
        requests: &requests,
        victims: &[],
        config: &config,
        budget,
        victim_capacity: 64,
    };
    let report = run_frame(&mut res, &input, &mut batch);
    // Edits get at least a quarter of the budget, loads at most the rest.
    assert!(
        u64::from(report.edited_uploaded) >= budget / 4 / brick_bytes,
        "{report:?}"
    );
    assert!(
        u64::from(report.loaded) * brick_bytes <= budget * 3 / 4 + brick_bytes,
        "{report:?}"
    );
    assert!(report.loaded >= 1, "{report:?}");
    // Edited bricks that did not fit are not resident, never stale.
    assert_eq!(
        report.edited_uploaded + report.edited_unlinked,
        dirty.len() as u32,
        "{report:?}"
    );
    apply_cpu(&mut pools, &batch);
    let segs: Vec<&[u32]> = pools.segments.iter().map(Vec::as_slice).collect();
    assert_eq!(res.verify(&snap, &pools.nodes, &segs, &pools.table), Ok(()));
    check_box(
        &snap,
        &pools,
        shape,
        VoxelBox::new(VoxelPos::new(-64, -16, -64), VoxelPos::new(64, 0, 64)),
    )
    .unwrap();
}

#[test]
fn stage_one_hysteresis_keeps_nodes_between_the_thresholds() {
    let snap = slab().snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let lod = LodTuning {
        detail_pixels: 1.0,
        hysteresis: 0.7,
        footprint_bias: 1.0,
    };
    let brick = res.resident_bricks()[0];
    let pixel_angle = 0.002;
    // Find an eye height where this brick projects between 0.7 and 1 pixel.
    let mut eye = [8.0f32, 0.0, 8.0];
    let lo = brick.brick_pos(shape.origin).min_voxel();
    eye[0] = lo.x as f32 + 8.0;
    eye[2] = lo.z as f32 + 8.0;
    let mut y = 10.0;
    let px_at = |y: f32| {
        projected_pixels(
            shape.origin,
            brick,
            &ViewPoint {
                eye: [eye[0], y, eye[2]],
                pixel_angle,
            },
        )
    };
    while px_at(y) > 0.85 {
        y += 1.0;
    }
    let view = ViewPoint {
        eye: [eye[0], y, eye[2]],
        pixel_angle,
    };
    let between = px_at(y);
    assert!((0.7..1.0).contains(&between), "{between}");

    // Resident and in the band: kept.
    let s = select(&res, &view, &lod);
    assert!(!s.evict.contains(&brick));
    // Evicted and in the band: not reloaded.
    let mut batch = UploadBatch::default();
    res.evict_brick(brick, &mut batch).unwrap();
    let s = select(&res, &view, &lod);
    assert!(!s.load.contains(&brick));
    // Closer than the detail threshold: loaded again.
    let near = ViewPoint {
        eye: [eye[0], lo.y as f32 + 20.0, eye[2]],
        pixel_angle,
    };
    assert!(select(&res, &near, &lod).load.contains(&brick));
    // Farther than the hysteresis band: a resident brick is evicted.
    res.load_brick(&snap, brick, &mut batch).unwrap();
    let far = ViewPoint {
        eye: [eye[0], y * 3.0, eye[2]],
        pixel_angle,
    };
    assert!(select(&res, &far, &lod).evict.contains(&brick));
}

/// From the default camera, stage 1 refines the debug scene down to resident
/// bricks within a few frames and the result stays consistent.
#[test]
fn stage_one_refines_the_debug_scene_from_the_default_camera() {
    let tree = world_render::internals::scene::build();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    let config = BrickCacheConfig::default();
    // 1280x720, 45 degree vertical field of view, at the origin.
    let view = ViewPoint {
        eye: [0.0, 0.0, 0.0],
        pixel_angle: 2.0 * (22.5f32.to_radians()).tan() / 720.0,
    };
    let mut batch = UploadBatch::default();
    for frame in 1..=20 {
        let input = FrameInput {
            source: &snap,
            frame,
            refresh: frame == 1,
            dirty: &[],
            view: Some(view),
            requests: &[],
            victims: &[],
            config: &config,
            budget: 24 << 20,
            victim_capacity: 64,
        };
        run_frame(&mut res, &input, &mut batch);
        apply_cpu(&mut pools, &batch);
    }
    let stats = res.stats();
    assert!(stats.bricks > 500, "{stats:?}");
    let segs: Vec<&[u32]> = pools.segments.iter().map(Vec::as_slice).collect();
    assert_eq!(res.verify(&snap, &pools.nodes, &segs, &pools.table), Ok(()));
}

#[test]
fn stage_two_loads_what_rays_request() {
    let snap = slab().snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &snap, &mut pools);
    let brick = res.resident_bricks()[3];
    let mut batch = UploadBatch::default();
    res.evict_brick(brick, &mut batch).unwrap();
    apply_cpu(&mut pools, &batch);
    assert!(!res.is_brick_resident(brick));

    // The request as the shader writes it, through the decoder.
    let mut words = vec![1, 0, 0, 0];
    words.extend(encode_request(brick, 50));
    let (requests, _) = world_render::internals::feedback::decode_requests(&words, 16, shape);
    let config = BrickCacheConfig {
        stage: Stage::RayGuided,
        ..Default::default()
    };
    let input = FrameInput {
        source: &snap,
        frame: 3,
        refresh: false,
        dirty: &[],
        view: None,
        requests: &requests,
        victims: &[],
        config: &config,
        budget: 1 << 20,
        victim_capacity: 64,
    };
    let report = run_frame(&mut res, &input, &mut batch);
    assert_eq!(report.loaded, 1);
    assert!(res.is_brick_resident(brick));
}

#[test]
fn coarse_samples_carry_the_level_and_representative_material() {
    let mut grid = DenseGrid::new(3, VoxelPos::new(0, 0, 0));
    for x in 0..5 {
        grid.set(VoxelPos::new(x, 0, 0), MaterialId(3));
    }
    grid.set(VoxelPos::new(1, 1, 1), MaterialId(7));
    let shape = Shape::of(&grid);
    let (mut res, mut pools) = cpu_cache(shape);
    make_resident(&mut res, &grid, &mut pools);
    let brick = NodeKey::containing([0, 0, 0], 1);
    let mut batch = UploadBatch::default();
    res.evict_brick(brick, &mut batch).unwrap();
    apply_cpu(&mut pools, &batch);
    let s = pools
        .with_view(|v| v.sample(shape.root_level(), [2, 2, 2]))
        .unwrap();
    assert_eq!(
        s,
        Sample::Coarse {
            level: 1,
            material: MaterialId(3)
        }
    );
    // The empty leaf next to it stays air.
    let s = pools
        .with_view(|v| v.sample(shape.root_level(), [8, 8, 8]))
        .unwrap();
    assert_eq!(s, Sample::Air);
}

#[test]
fn shader_constants_match_rust() {
    let text = std::fs::read_to_string(common::assets_dir().join("shaders/common/format.wesl"))
        .expect("format.wesl");
    let mut found = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("const ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let name = name.split(':').next().unwrap().trim().to_string();
        let v = value
            .trim()
            .trim_end_matches(';')
            .trim()
            .trim_end_matches('u');
        let parsed = if let Some(hex) = v.strip_prefix("0x") {
            u32::from_str_radix(hex, 16)
        } else {
            v.parse::<u32>()
        };
        if let Ok(n) = parsed {
            found.insert(name, n);
        }
    }
    for (name, value) in world_render::internals::shader_constants() {
        assert_eq!(
            found.get(name),
            Some(&value),
            "format.wesl: {name} should be {value:#x}"
        );
    }
}

#[test]
fn the_shipped_config_is_valid_and_matches_the_defaults() {
    use world_core::ConfigAsset;
    let text = std::fs::read_to_string(common::assets_dir().join(BrickCacheConfig::PATH))
        .expect("brick_cache.ron");
    let parsed: BrickCacheConfig =
        world_reload::parse_config(&text).unwrap_or_else(|e| panic!("brick_cache.ron: {e}"));
    assert_eq!(parsed, BrickCacheConfig::default());
}

/// Loading bricks in parallel gives exactly the pools that loading them one
/// by one gives: ids are handed out in order before encoding.
#[test]
fn parallel_brick_loading_matches_sequential_loading() {
    let mut tree = VoxelTree::new();
    for i in 0..3000i32 {
        let p = VoxelPos::new((i * 37) % 120 - 60, (i * 11) % 40 - 20, (i * 53) % 120 - 60);
        tree.set(p, MaterialId(1 + (i % 7) as u16)).unwrap();
    }
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let run = |parallel: bool| {
        let (mut res, mut pools) = cpu_cache(shape);
        let mut batch = UploadBatch::default();
        res.reset(&snap, &mut batch);
        loop {
            let unexpanded: Vec<NodeKey> = res
                .keys()
                .into_iter()
                .filter(|k| k.level >= 2 && !res.is_expanded(*k))
                .collect();
            if unexpanded.is_empty() {
                break;
            }
            for k in unexpanded {
                let _ = res.expand(&snap, k, &mut batch);
            }
        }
        let mut bricks: Vec<NodeKey> = res.keys().into_iter().filter(|k| k.level == 1).collect();
        bricks.sort_unstable();
        if parallel {
            let (loaded, full) = res.load_bricks(&snap, &bricks, 8, |_| true, &mut batch);
            assert!(!full);
            assert!(loaded > 100, "{loaded}");
        } else {
            for &k in &bricks {
                let _ = res.load_brick(&snap, k, &mut batch);
            }
        }
        apply_cpu(&mut pools, &batch);
        let segs: Vec<&[u32]> = pools.segments.iter().map(Vec::as_slice).collect();
        assert_eq!(res.verify(&snap, &pools.nodes, &segs, &pools.table), Ok(()));
        (
            pools.nodes.clone(),
            pools.segments.clone(),
            pools.table.clone(),
        )
    };
    assert!(
        run(true) == run(false),
        "parallel and sequential pools differ"
    );
}
