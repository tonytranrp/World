//! The brick cache on the GPU: upload, apply and eviction cycles keep the
//! pools equal to the CPU model and consistent with the residency table; a
//! destroyed brick never stays visible; ray requests page bricks in.

mod common;

use common::*;
use world_core::coords::path::child_slot;
use world_core::tree::{NodeKind, NodeView};
use world_core::{BrickPos, Edit, MaterialId, TreeSnapshot, VoxelBox, VoxelPos, VoxelTree};
use world_render::internals::batch::{CpuPools, UploadBatch};
use world_render::internals::evict::{EvictParams, choose_victims};
use world_render::internals::feedback::{decode_requests, decode_victims};
use world_render::internals::format::decode::Sample;
use world_render::internals::frame::{FrameInput, run_frame};
use world_render::internals::gbuffer::GpuHit;
use world_render::internals::key::{NodeKey, Shape, view_at};
use world_render::internals::residency::Residency;
use world_render::internals::{BrickCacheConfig, Stage};

/// Deterministic pseudo-random numbers.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }

    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo) as u32) as i32
    }
}

const REGION: [i32; 6] = [-48, -24, -48, 48, 24, 48];

fn scene() -> VoxelTree {
    let mut t = VoxelTree::new();
    t.fill_box(
        VoxelBox::new(VoxelPos::new(-48, -24, -48), VoxelPos::new(48, -8, 48)),
        MaterialId(1),
    )
    .unwrap();
    t.fill_box(
        VoxelBox::new(VoxelPos::new(-20, -8, -20), VoxelPos::new(20, 12, 20)),
        MaterialId(2),
    )
    .unwrap();
    for i in 0..48 {
        t.set(VoxelPos::new(-40 + i, -8, 10 - i / 2), MaterialId(3))
            .unwrap();
    }
    t
}

/// Pools read back must equal the CPU model, word for word.
fn assert_pools_equal(gpu_pools: &CpuPools, cpu_pools: &CpuPools, step: usize) {
    let first_node = gpu_pools
        .nodes
        .iter()
        .zip(&cpu_pools.nodes)
        .position(|(a, b)| a != b);
    assert_eq!(first_node, None, "step {step}: node pools differ");
    for (s, (g, c)) in gpu_pools
        .segments
        .iter()
        .zip(&cpu_pools.segments)
        .enumerate()
    {
        let first = g.iter().zip(c).position(|(a, b)| a != b);
        assert_eq!(first, None, "step {step}: segment {s} differs");
    }
    let first = gpu_pools
        .table
        .iter()
        .zip(&cpu_pools.table)
        .position(|(a, b)| a != b);
    assert_eq!(first, None, "step {step}: brick tables differ");
}

/// Every voxel of the region, decoded from `pools`, agrees with the source:
/// exact voxels equal it, air is air, coarse cells contain solid voxels now.
fn assert_never_stale(source: &TreeSnapshot, pools: &CpuPools, step: usize) {
    let shape = Shape::of(source);
    for z in REGION[2]..REGION[5] {
        for y in REGION[1]..REGION[4] {
            for x in REGION[0]..REGION[3] {
                let pos = VoxelPos::new(x, y, z);
                let local = shape.local(pos).unwrap();
                let sample = pools
                    .with_view(|v| v.sample(shape.root_level(), local))
                    .unwrap_or_else(|e| panic!("step {step} {pos:?}: {e}"));
                let truth = source.get(pos);
                let ok = match sample {
                    Sample::Exact(m) => m == truth,
                    Sample::Air => truth.is_air(),
                    Sample::Coarse { level, .. } => {
                        let node = view_at(source, NodeKey::containing(local, level));
                        node.child(child_slot(local, level)).kind() != NodeKind::Air
                    }
                };
                assert!(
                    ok,
                    "step {step} {pos:?}: pools say {sample:?}, source {truth:?}"
                );
            }
        }
    }
}

fn verify(res: &mut Residency, source: &TreeSnapshot, pools: &CpuPools, step: usize) {
    let segs: Vec<&[u32]> = pools.segments.iter().map(Vec::as_slice).collect();
    if let Err(e) = res.verify(source, &pools.nodes, &segs, &pools.table) {
        panic!("step {step}: {e}");
    }
}

#[test]
fn gpu_upload_apply_and_evict_cycles_keep_the_pools_equal_to_the_cpu_model() {
    let Some(gpu) = gpu() else {
        return;
    };
    let pipelines = Pipelines::new(&gpu.device);
    let mut tree = scene();
    let shape = Shape::of(&tree);
    let (mut res, mut cpu_pools) = cpu_cache(shape);
    let mut cache = GpuCache::new(&gpu, &pipelines, small_layout());
    for batch in make_resident(&mut res, &tree.snapshot(), &mut cpu_pools) {
        cache.apply(&gpu, &pipelines, &batch);
    }
    assert_pools_equal(&cache.read(&gpu), &cpu_pools, 0);

    let mut rng = Rng(0x5EED_1234_ABCD);
    let mut batch = UploadBatch::default();
    for step in 1..=16usize {
        res.set_frame(step as u32 + 1);
        // A few edits: single voxels and a box carve or fill.
        let mut dirty = Vec::new();
        let edits: Vec<Edit> = (0..rng.range(1, 12))
            .map(|_| {
                Edit::new(
                    VoxelPos::new(rng.range(-48, 48), rng.range(-24, 24), rng.range(-48, 48)),
                    MaterialId(rng.range(0, 6) as u16),
                )
            })
            .collect();
        dirty.extend_from_slice(tree.apply(&edits).unwrap().as_slice());
        if step % 3 == 0 {
            let lo = VoxelPos::new(rng.range(-48, 30), rng.range(-24, 10), rng.range(-48, 30));
            let region = VoxelBox::new(lo, VoxelPos::new(lo.x + 17, lo.y + 9, lo.z + 13));
            let m = MaterialId(if step % 2 == 0 { 0 } else { 4 });
            tree.fill_box(region, m).unwrap();
            for bz in BrickPos::containing(lo).z..=BrickPos::containing(region.end()).z {
                for by in BrickPos::containing(lo).y..=BrickPos::containing(region.end()).y {
                    for bx in BrickPos::containing(lo).x..=BrickPos::containing(region.end()).x {
                        dirty.push(BrickPos::new(bx, by, bz));
                    }
                }
            }
        }
        let snap = tree.snapshot();
        let keys: Vec<NodeKey> = dirty.iter().filter_map(|&b| shape.brick_key(b)).collect();
        let upload = step % 4 != 1;
        res.apply_edits(&snap, &keys, |_| upload, &mut batch);
        // Some structural churn.
        let all = res.keys();
        for _ in 0..3 {
            let k = all[rng.next() as usize % all.len()];
            let _ = match rng.next() % 4 {
                0 => res.collapse(k, &mut batch),
                1 => res.evict_brick(k, &mut batch),
                2 => res.load_brick(&snap, k, &mut batch).map(|_| ()),
                _ => res.expand(&snap, k, &mut batch).map(|_| ()),
            };
        }
        apply_cpu(&mut cpu_pools, &batch);
        cache.apply(&gpu, &pipelines, &batch);
        batch.clear();

        let gpu_pools = cache.read(&gpu);
        assert_pools_equal(&gpu_pools, &cpu_pools, step);
        verify(&mut res, &snap, &gpu_pools, step);
        assert_never_stale(&snap, &gpu_pools, step);
    }
}

#[test]
fn gpu_eviction_chooses_the_oldest_bricks_and_the_cpu_frees_them() {
    let Some(gpu) = gpu() else {
        return;
    };
    let pipelines = Pipelines::new(&gpu.device);
    let tree = scene();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut cpu_pools) = cpu_cache(shape);
    let mut cache = GpuCache::new(&gpu, &pipelines, small_layout());
    for batch in make_resident(&mut res, &snap, &mut cpu_pools) {
        cache.apply(&gpu, &pipelines, &batch);
    }
    // Reload bricks at different frames so their ages differ.
    let bricks = res.resident_bricks();
    let mut batch = UploadBatch::default();
    for (i, &k) in bricks.iter().enumerate() {
        res.set_frame(10 + (i as u32 % 120));
        res.evict_brick(k, &mut batch).unwrap();
        res.load_brick(&snap, k, &mut batch).unwrap();
    }
    apply_cpu(&mut cpu_pools, &batch);
    cache.apply(&gpu, &pipelines, &batch);
    batch.clear();

    let layout = cache.pools.layout.clone();
    let params = EvictParams {
        frame: 200,
        min_age: 30,
        target: (bricks.len() / 4) as u32,
        id_count: layout.brick_ids,
        capacity: layout.victim_capacity,
        _pad: [0; 3],
    };
    let evict_layout = &pipelines.evict_layout;
    let pass = world_render::internals::evict::pass::EvictPass::new(
        &gpu.device,
        evict_layout,
        &cache.pools,
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    pass.record(
        &gpu.queue,
        &mut encoder,
        &cache.pools,
        &pipelines.evict,
        &params,
    );
    gpu.queue.submit([encoder.finish()]);
    let words = read_words(&gpu, &cache.pools.victims);
    let (victims, stats) = decode_victims(&words, layout.victim_capacity, layout.brick_ids);
    assert_eq!((stats.invalid, stats.overflow), (0, 0));

    let expected = choose_victims(&cpu_pools.table, &params);
    let chosen: Vec<u32> = victims.iter().map(|v| v.id).collect();
    assert_eq!(chosen, expected);
    assert!(chosen.len() >= params.target as usize);
    // Each victim is at least 30 frames old.
    for v in &victims {
        let last = cpu_pools.table[v.id as usize * 2 + 1];
        assert!(200 - last >= 30);
    }

    // The CPU frees them, checking the block each was chosen with.
    res.set_frame(201);
    let mut freed = 0;
    for v in &victims {
        freed += u32::from(res.evict_victim(v.id, v.block, &mut batch));
    }
    assert_eq!(freed as usize, victims.len());
    // A stale victim (same id, other block) is ignored.
    assert!(!res.evict_victim(victims[0].id, victims[0].block ^ 1, &mut batch));
    apply_cpu(&mut cpu_pools, &batch);
    cache.apply(&gpu, &pipelines, &batch);
    let gpu_pools = cache.read(&gpu);
    assert_pools_equal(&gpu_pools, &cpu_pools, 1);
    verify(&mut res, &snap, &gpu_pools, 1);
    assert_never_stale(&snap, &gpu_pools, 1);
}

fn camera_rays(width: u32, height: u32) -> Vec<([f32; 3], [f32; 3])> {
    let camera = world_testkit::hits::Camera {
        eye: [3.3, 30.7, -70.2],
        forward: [0.0, -0.45, 1.0],
        up: [0.0, 1.0, 0.0],
        vertical_fov_radians: 1.0,
    };
    camera
        .rays(width, height)
        .into_iter()
        .map(|(_, r)| (r.origin, r.dir))
        .collect()
}

/// A brick changed beyond the upload budget is unlinked in the same batch:
/// rays see the new coarse cells, never the old voxels. Its rays then
/// request it, stage 2 loads it, and the next trace is exact again.
#[test]
fn gpu_destroyed_bricks_never_stay_visible_and_requests_page_them_back() {
    let Some(gpu) = gpu() else {
        return;
    };
    let pipelines = Pipelines::new(&gpu.device);
    let mut tree = scene();
    let shape = Shape::of(&tree);
    let (mut res, mut cpu_pools) = cpu_cache(shape);
    let mut cache = GpuCache::new(&gpu, &pipelines, small_layout());
    for batch in make_resident(&mut res, &tree.snapshot(), &mut cpu_pools) {
        cache.apply(&gpu, &pipelines, &batch);
    }

    // Carve a crater through the tower with no upload budget.
    let crater = VoxelBox::new(VoxelPos::new(-12, -8, -24), VoxelPos::new(12, 12, 0));
    tree.fill_box(crater, MaterialId::AIR).unwrap();
    let mut dirty = Vec::new();
    for bz in -2..=0 {
        for by in -1..=0 {
            for bx in -1..=0 {
                dirty.push(BrickPos::new(bx, by, bz));
            }
        }
    }
    let snap = tree.snapshot();
    let config = BrickCacheConfig {
        stage: Stage::RayGuided,
        ..Default::default()
    };
    let mut batch = UploadBatch::default();
    let input = FrameInput {
        source: &snap,
        frame: 5,
        refresh: false,
        dirty: &dirty,
        view: None,
        requests: &[],
        victims: &[],
        config: &config,
        budget: 64, // nothing fits but the header
        victim_capacity: 64,
    };
    let report = run_frame(&mut res, &input, &mut batch);
    assert_eq!(report.edited_uploaded, 0);
    assert!(report.edited_unlinked > 0, "{report:?}");
    cache.apply(&gpu, &pipelines, &batch);

    let (w, h) = (48, 32);
    let rays = camera_rays(w, h);
    let setup = TraceSetup {
        shape,
        frame: 5,
        pixel_angle: 0.01,
        footprint_bias: 0.0,
    };
    let check = |hits: &[GpuHit]| -> usize {
        let mut coarse = 0;
        for hit in hits.iter().filter(|h| h.is_hit()) {
            let local = shape.local(hit.voxel).unwrap();
            if hit.level == 0 {
                assert_eq!(snap.get(hit.voxel), hit.material, "stale voxel {hit:?}");
            } else {
                coarse += 1;
                let node = view_at(&snap, NodeKey::containing(local, hit.level));
                assert_ne!(
                    node.child(child_slot(local, hit.level)).kind(),
                    NodeKind::Air,
                    "coarse hit on a cell the edit emptied: {hit:?}"
                );
            }
        }
        coarse
    };
    let hits = gpu_trace(&gpu, &pipelines, &cache, &rays, w, setup);
    assert!(check(&hits) > 0, "no ray reached the destroyed bricks");

    // The rays requested the unlinked bricks; stage 2 loads them.
    let feedback = read_words(&gpu, &cache.pools.feedback);
    let layout = cache.pools.layout.clone();
    let (requests, _) = decode_requests(
        &feedback[..layout.request_block_words() as usize],
        layout.request_capacity,
        shape,
    );
    assert!(!requests.is_empty());
    let input = FrameInput {
        source: &snap,
        frame: 6,
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
    assert_eq!(report.loaded as usize, requests.len(), "{report:?}");
    cache.apply(&gpu, &pipelines, &batch);
    let hits = gpu_trace(
        &gpu,
        &pipelines,
        &cache,
        &rays,
        w,
        TraceSetup { frame: 6, ..setup },
    );
    assert_eq!(check(&hits), 0, "still coarse after paging in");
}
