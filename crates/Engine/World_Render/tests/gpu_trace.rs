//! The primary trace on the GPU against the World_Testkit brute-force DDA.
//!
//! Exactness rule (decisions 0005 and 0007): with a fully resident pool and
//! the footprint bias at full resolution, every pixel's integer hit (voxel,
//! face, material) equals the CPU reference's, and the distance agrees
//! within [`T_TOLERANCE`]. WGSL division may be off by a few ulps (and Metal
//! compiles with fast math), so distances are not compared bit for bit.

mod common;

use common::*;
use world_core::tree::VoxelSource;
use world_core::{BrickPos, DenseGrid, MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_render::internals::batch::UploadBatch;
use world_render::internals::gbuffer::{GpuHit, HIT_FLAG_FOOTPRINT, HIT_FLAG_RESIDENCY};
use world_render::internals::key::{NodeKey, Shape};
use world_render::internals::residency::Residency;
use world_render::internals::trace;
use world_testkit::dense::{Bounds, DenseWorld, VoxelModel};
use world_testkit::hits::{Camera, Face, Hit, HitRecord, compare_hits, trace_rays};

/// Distance tolerance in voxels between the GPU and the CPU reference.
const T_TOLERANCE: f32 = 1e-3;

fn face(i: u32) -> Face {
    [
        Face::NegX,
        Face::PosX,
        Face::NegY,
        Face::PosY,
        Face::NegZ,
        Face::PosZ,
        Face::Inside,
    ]
    .get(i as usize)
    .copied()
    .unwrap_or(Face::Inside)
}

fn as_record(pixel: (u32, u32), h: &GpuHit) -> HitRecord {
    HitRecord {
        pixel,
        hit: h.is_hit().then_some(Hit {
            pos: h.voxel,
            material: h.material,
            face: face(h.face),
            t: h.t,
        }),
    }
}

/// A scene in both the source under test and the dense reference.
fn copy_into_dense<S: VoxelSource>(source: &S, bounds: Bounds) -> DenseWorld {
    let mut dense = DenseWorld::new(bounds);
    let b = bounds.as_box();
    for z in b.min().z..b.end().z {
        for y in b.min().y..b.end().y {
            for x in b.min().x..b.end().x {
                let p = VoxelPos::new(x, y, z);
                let m = source.get(p);
                if !m.is_air() {
                    dense.set(p, m);
                }
            }
        }
    }
    dense
}

/// Walls, a hollow box, a few lone voxels and a sloped staircase.
fn tree_scene() -> (VoxelTree, Bounds) {
    let mut t = VoxelTree::new();
    let fill = |t: &mut VoxelTree, a: [i32; 3], b: [i32; 3], m: u16| {
        t.fill_box(
            VoxelBox::new(VoxelPos::from(a), VoxelPos::from(b)),
            MaterialId(m),
        )
        .unwrap();
    };
    fill(&mut t, [-40, -20, -40], [40, -16, 40], 1);
    fill(&mut t, [-40, -16, 30], [40, 20, 34], 2);
    fill(&mut t, [-10, -16, -10], [10, 4, 10], 3);
    fill(&mut t, [-8, -14, -8], [8, 2, 8], 0);
    // One brick with full leaves of two materials: 7 is its default (no
    // words), 8 its other full leaves (one payload each, decision 0021).
    fill(&mut t, [16, -16, 16], [32, -8, 28], 7);
    fill(&mut t, [16, -8, 16], [24, 0, 28], 8);
    // A brick of 9 whose top ends halfway up a row of leaves: its partial
    // leaves of the default are leaf masks without payloads (decision
    // 0025). Where 10 meets 9 off the leaf grid, leaves of two materials
    // (palette blocks without a header); where 11 joins, one of three.
    fill(&mut t, [-32, -16, -32], [-16, -6, -16], 9);
    fill(&mut t, [-32, -10, -24], [-26, -6, -16], 10);
    t.set(VoxelPos::new(-27, -7, -18), MaterialId(11)).unwrap();
    for i in 0..20 {
        fill(&mut t, [20 + i, -16, -30], [21 + i, -16 + i, -10], 4);
        t.set(
            VoxelPos::new(-30 + 3 * i, 10 - i, -20 + 2 * i),
            MaterialId(5),
        )
        .unwrap();
        t.set(VoxelPos::new(-25 + i, -15, 25 - i), MaterialId(6))
            .unwrap();
    }
    (t, Bounds::new(VoxelPos::new(-40, -20, -40), [80, 54, 80]))
}

fn grid_scene() -> (DenseGrid, Bounds) {
    let origin = VoxelPos::new(16, -32, 48);
    let mut g = DenseGrid::new(3, origin);
    let mut seed = 0x9E37_79B9u32;
    for _ in 0..4000 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let p = VoxelPos::new(
            origin.x + (seed % 64) as i32,
            origin.y + ((seed >> 6) % 24) as i32,
            origin.z + ((seed >> 12) % 64) as i32,
        );
        g.set(p, MaterialId(1 + (seed >> 20) as u16 % 7));
    }
    for x in 0..64 {
        for z in 0..64 {
            g.set(
                VoxelPos::new(origin.x + x, origin.y, origin.z + z),
                MaterialId(2),
            );
        }
    }
    (g, Bounds::new(origin, [64, 64, 64]))
}

/// Uploads every node and brick of `source` to a fresh GPU cache.
fn resident_cache<S: VoxelSource>(
    gpu: &Gpu,
    pipelines: &Pipelines,
    source: &S,
) -> (Residency, GpuCache) {
    let shape = Shape::of(source);
    let (mut res, mut pools) = cpu_cache(shape);
    let mut cache = GpuCache::new(gpu, pipelines, small_layout());
    for batch in make_resident(&mut res, source, &mut pools) {
        cache.apply(gpu, pipelines, &batch);
    }
    (res, cache)
}

fn cameras_for(bounds: Bounds) -> Vec<Camera> {
    let lo = bounds.min.to_array().map(|v| v as f32);
    let s = bounds.size.map(|v| v as f32);
    let c = [lo[0] + s[0] / 2.0, lo[1] + s[1] / 2.0, lo[2] + s[2] / 2.0];
    vec![
        // Outside, looking in from the front, slightly above.
        Camera {
            eye: [c[0] + 0.37, c[1] + s[1] * 0.3, lo[2] - 30.6],
            forward: [0.03, -0.25, 1.0],
            up: [0.0, 1.0, 0.0],
            vertical_fov_radians: 1.0,
        },
        // Inside the scene, looking diagonally down.
        Camera {
            eye: [c[0] - 3.3, c[1] + 2.7, c[2] - 5.1],
            forward: [0.6, -0.5, 0.7],
            up: [0.0, 1.0, 0.0],
            vertical_fov_radians: 1.2,
        },
        // From a corner along a near-diagonal, negative on every axis.
        Camera {
            eye: [
                lo[0] + s[0] + 20.2,
                lo[1] + s[1] + 15.9,
                lo[2] + s[2] + 18.4,
            ],
            forward: [-1.0, -0.8, -0.9],
            up: [0.0, 1.0, 0.0],
            vertical_fov_radians: 0.9,
        },
    ]
}

const W: u32 = 64;
const H: u32 = 48;

fn compare_scene<S: VoxelSource>(source: &S, bounds: Bounds, label: &str) {
    let Some(gpu) = gpu() else {
        return;
    };
    let pipelines = Pipelines::new(&gpu.device);
    let (_res, cache) = resident_cache(&gpu, &pipelines, source);
    let dense = copy_into_dense(source, bounds);
    let setup = TraceSetup {
        shape: Shape::of(source),
        frame: 1,
        pixel_angle: 0.01,
        footprint_bias: 0.0,
    };
    for (i, camera) in cameras_for(bounds).iter().enumerate() {
        let rays = camera.rays(W, H);
        let cpu = trace_rays(&dense, &rays);
        let input: Vec<_> = rays.iter().map(|(_, r)| (r.origin, r.dir)).collect();
        let hits = gpu_trace(&gpu, &pipelines, &cache, &input, W, setup);
        let gpu_records: Vec<HitRecord> = rays
            .iter()
            .zip(&hits)
            .map(|((p, _), h)| as_record(*p, h))
            .collect();
        let solid = cpu.iter().filter(|r| r.hit.is_some()).count();
        assert!(
            solid > 100,
            "{label} camera {i}: only {solid} pixels hit anything"
        );
        if let Err(e) = compare_hits(&cpu, &gpu_records, T_TOLERANCE) {
            panic!("{label} camera {i} on {}: {e}", gpu.info.name);
        }
        assert!(
            hits.iter().all(|h| h.level == 0),
            "{label}: coarse hit at full residency"
        );
    }
}

#[test]
fn gpu_trace_matches_the_brute_force_dda_on_a_world_tree() {
    let (tree, bounds) = tree_scene();
    // The scene covers every brick and palette layout (decisions 0021 and
    // 0025): long headers, other full leaves, default partial leaves, and
    // palette blocks of two materials (one bit) and of three (two bits).
    let b = tree.memory_breakdown();
    assert!(b.long_headers > 0 && b.full_records > 0 && b.default_partial_leaves > 0);
    assert!(b.palette_words[0] > 0 && b.palette_words[1] > 0, "{b:?}");
    compare_scene(&tree.snapshot(), bounds, "world tree");
}

#[test]
fn gpu_trace_matches_the_brute_force_dda_on_a_dense_grid() {
    let (grid, bounds) = grid_scene();
    compare_scene(&grid, bounds, "dense grid");
}

/// Evicted bricks are drawn at leaf-mask level (cells of 4 voxels) with the
/// brick's representative material, flagged as residency-limited, and are
/// requested once per frame; everything else stays exact.
#[test]
fn gpu_coarse_residency_hits_at_leaf_level_with_the_representative_material() {
    let Some(gpu) = gpu() else {
        return;
    };
    let pipelines = Pipelines::new(&gpu.device);
    let (tree, bounds) = tree_scene();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut cache) = resident_cache(&gpu, &pipelines, &snap);

    // Evict every third brick.
    let mut batch = UploadBatch::default();
    let evicted: Vec<NodeKey> = res.resident_bricks().into_iter().step_by(3).collect();
    for &k in &evicted {
        res.evict_brick(k, &mut batch).unwrap();
    }
    cache.apply(&gpu, &pipelines, &batch);

    // Reference: in each evicted brick, every non-empty leaf becomes a solid
    // 4³ block of the brick's representative material.
    let mut dense = copy_into_dense(&snap, bounds);
    let mut evicted_bricks = std::collections::BTreeSet::new();
    for &k in &evicted {
        let material = res.record(k).unwrap().material();
        let brick = k.brick_pos(shape.origin);
        evicted_bricks.insert(brick);
        let lo = brick.min_voxel();
        for lz in 0..4 {
            for ly in 0..4 {
                for lx in 0..4 {
                    let cell = [lo.x + 4 * lx, lo.y + 4 * ly, lo.z + 4 * lz];
                    let any = (0..64).any(|i| {
                        !snap
                            .get(VoxelPos::new(
                                cell[0] + (i & 3),
                                cell[1] + ((i >> 2) & 3),
                                cell[2] + (i >> 4),
                            ))
                            .is_air()
                    });
                    for i in 0..64 {
                        let p = VoxelPos::new(
                            cell[0] + (i & 3),
                            cell[1] + ((i >> 2) & 3),
                            cell[2] + (i >> 4),
                        );
                        if bounds.contains(p) {
                            dense.set(p, if any { material } else { MaterialId::AIR });
                        }
                    }
                }
            }
        }
    }

    let setup = TraceSetup {
        shape,
        frame: 7,
        pixel_angle: 0.01,
        footprint_bias: 0.0,
    };
    let camera = &cameras_for(bounds)[0];
    let rays = camera.rays(W, H);
    let cpu = trace_rays(&dense, &rays);
    let input: Vec<_> = rays.iter().map(|(_, r)| (r.origin, r.dir)).collect();
    let hits = gpu_trace(&gpu, &pipelines, &cache, &input, W, setup);
    let gpu_records: Vec<HitRecord> = rays
        .iter()
        .zip(&hits)
        .map(|((p, _), h)| as_record(*p, h))
        .collect();
    if let Err(e) = compare_hits(&cpu, &gpu_records, T_TOLERANCE) {
        panic!("coarse residency on {}: {e}", gpu.info.name);
    }
    let mut coarse = 0;
    for h in hits.iter().filter(|h| h.is_hit()) {
        let in_evicted = evicted_bricks.contains(&BrickPos::containing(h.voxel));
        if in_evicted {
            assert_eq!(
                (h.level, h.flags & HIT_FLAG_RESIDENCY),
                (1, HIT_FLAG_RESIDENCY),
                "{h:?}"
            );
            coarse += 1;
        } else {
            assert_eq!(h.level, 0, "{h:?}");
        }
    }
    assert!(coarse > 0, "no ray reached an evicted brick");

    // Requests: only evicted bricks, each once, after this frame.
    let feedback = read_words(&gpu, &cache.pools.feedback);
    let layout = &cache.pools.layout;
    let (requests, stats) = world_render::internals::feedback::decode_requests(
        &feedback[..layout.request_block_words() as usize],
        layout.request_capacity,
        shape,
    );
    assert_eq!(stats.invalid, 0);
    assert_eq!(
        stats.written as usize,
        requests.len(),
        "a node was requested twice"
    );
    assert!(!requests.is_empty());
    for r in &requests {
        assert!(
            evicted.contains(&r.key),
            "requested {:?}, which is resident",
            r.key
        );
    }
    // Resident bricks the rays used carry this frame's stamp.
    let table = read_words(&gpu, &cache.pools.table);
    assert!(
        table.chunks_exact(2).any(|e| e[1] == 7),
        "no brick was stamped"
    );
}

/// A large footprint bias stops rays at coarse cells; each such cell holds
/// solid voxels, and a bias of 0 stops nowhere.
#[test]
fn gpu_footprint_stops_at_coarse_cells_that_hold_voxels() {
    let Some(gpu) = gpu() else {
        return;
    };
    let pipelines = Pipelines::new(&gpu.device);
    let (tree, bounds) = tree_scene();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (_res, cache) = resident_cache(&gpu, &pipelines, &snap);
    let camera = &cameras_for(bounds)[0];
    let rays = camera.rays(W, H);
    let input: Vec<_> = rays.iter().map(|(_, r)| (r.origin, r.dir)).collect();
    let setup = TraceSetup {
        shape,
        frame: 1,
        pixel_angle: 0.02,
        footprint_bias: 8.0,
    };
    let hits = gpu_trace(&gpu, &pipelines, &cache, &input, W, setup);
    let mut coarse = 0;
    for h in hits.iter().filter(|h| h.is_hit()) {
        if h.flags & HIT_FLAG_FOOTPRINT != 0 {
            coarse += 1;
            assert!(h.level >= 1);
            let span = 1i32 << (2 * h.level);
            let lo = h.voxel.to_array().map(|c| c.div_euclid(span) * span);
            let any = (0..span).any(|z| {
                (0..span).any(|y| {
                    (0..span).any(|x| {
                        !snap
                            .get(VoxelPos::new(lo[0] + x, lo[1] + y, lo[2] + z))
                            .is_air()
                    })
                })
            });
            assert!(any, "footprint hit on an empty cell: {h:?}");
        }
    }
    assert!(coarse > 0, "the bias never stopped a ray");
}

/// A 256 x 144 trace over the tree scene, every ray from the origin down -z
/// (camera mode with identity matrices): GPU work to time.
fn timing_trace(gpu: &Gpu) -> (Pipelines, GpuCache, wgpu::BindGroup, [wgpu::Buffer; 2]) {
    use wgpu::util::DeviceExt;
    use world_render::internals::trace::MODE_CAMERA;
    let pipelines = Pipelines::new(&gpu.device);
    let (tree, _) = tree_scene();
    let snap = tree.snapshot();
    let (_res, cache) = resident_cache(gpu, &pipelines, &snap);
    let shape = Shape::of(&snap);
    let hits = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timing hits"),
        size: u64::from(256 * 144 * world_render::internals::gbuffer::HIT_WORDS) * 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let layout = &cache.pools.layout;
    let o = shape.origin;
    let params = trace::TraceParams {
        clip_from_world: identity(),
        world_from_clip: identity(),
        eye: [0.0; 4],
        origin: [o.x, o.y, o.z, shape.root_level() as i32],
        size: [256, 144, 1, MODE_CAMERA],
        lod: [0.01, 0.0, 64.0, 0.0],
        limits: [
            layout.request_capacity,
            layout.stamps_offset(),
            trace::DEFAULT_MAX_ITERATIONS,
            0,
        ],
    };
    let params = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("timing params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let bind_group = trace::trace_bind_group(
        &gpu.device,
        &pipelines.trace_layout,
        &cache.pools,
        &hits,
        &params,
    );
    (pipelines, cache, bind_group, [hits, params])
}

/// The timestamp timer resolves and reads back a pass's time a few frames
/// late, never waiting; each pass is timed once per frame.
#[test]
fn gpu_pass_timer_reads_back_trace_times_without_waiting() {
    use world_render::internals::gpu_timing::{GpuPassTimer, TIMING_SLOTS, TimedPass};
    let Some(gpu) = gpu() else {
        return;
    };
    let Some(mut timer) = GpuPassTimer::new(&gpu.device, &gpu.queue) else {
        println!(
            "{} has no TIMESTAMP_QUERY: no timestamp timer (the sampled timer is used)",
            gpu.info.name
        );
        return;
    };
    let (pipelines, _cache, bind_group, _buffers) = timing_trace(&gpu);
    let frames = 2 * TIMING_SLOTS as u64;
    for _ in 0..frames {
        timer.begin_frame(&gpu.device);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let writes = timer.compute_writes(TimedPass::Trace);
        let timed = writes.is_some();
        trace::record_trace(
            &mut encoder,
            &pipelines.trace,
            &bind_group,
            256,
            144,
            writes,
        );
        if timed {
            assert!(
                timer.compute_writes(TimedPass::Trace).is_none(),
                "a pass is timed once per frame"
            );
        }
        timer.resolve(&mut encoder);
        gpu.queue.submit([encoder.finish()]);
        timer.after_submit();
    }
    wait(&gpu);
    timer.collect(&gpu.device);
    let times = timer.take().expect("no frame was read back");
    let trace = times.pass(TimedPass::Trace);
    assert_eq!(u64::from(trace.frames) + timer.skipped, frames);
    assert!(trace.frames >= 1 && trace.mean_ms() > 0.0, "{times}");
    assert!(trace.max_ms < 1000.0, "{times}");
    assert_eq!(times.pass(TimedPass::Apply).frames, 0);
    assert_eq!(timer.failed, 0);
}

/// The sampled timer, for devices without timestamps, times a pass
/// submitted alone.
#[test]
fn gpu_sampled_timer_times_a_pass_submitted_alone() {
    use world_render::internals::gpu_timing::{SampledPassTimer, TimedPass};
    let Some(gpu) = gpu() else {
        return;
    };
    let (pipelines, _cache, bind_group, _buffers) = timing_trace(&gpu);
    let mut sampled = SampledPassTimer::default();
    sampled.every = 1;
    sampled.begin_frame();
    assert!(sampled.wants(TimedPass::Trace));
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    trace::record_trace(&mut encoder, &pipelines.trace, &bind_group, 256, 144, None);
    let mut flushed = false;
    sampled.time_alone(
        TimedPass::Trace,
        &gpu.device,
        &gpu.queue,
        || flushed = true,
        encoder,
    );
    assert!(flushed);
    assert!(!sampled.wants(TimedPass::Trace));
    sampled.begin_frame();
    let times = sampled.take().expect("the sampled frame was not added");
    let trace = times.pass(TimedPass::Trace);
    assert_eq!(trace.frames, 1);
    assert!((0.0..1000.0).contains(&trace.max_ms), "{times}");
}
