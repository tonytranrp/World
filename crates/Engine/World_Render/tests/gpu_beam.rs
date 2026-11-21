//! The beam prepass (decision 0023) against the trace without it.
//!
//! The prepass may only move where camera rays start, never what they hit:
//! every hit record must be bit for bit the one the trace writes from the
//! camera (the step count aside), and the frame's brick requests and brick
//! stamps must be the same. The scene is built to catch a beam that is not
//! conservative: pillars one voxel wide that cover a fraction of a pixel and
//! fall between tile corners, a one-voxel wall and a one-voxel plate seen
//! edge-on, lone voxels in the air, grazing views of a large floor from just
//! above it, partial tiles on both image edges, coarse footprint hits and
//! evicted bricks (residency hits and requests).
//!
//! Breaking the beam on purpose (its spread set to 0, so it covers only the
//! tile's centre ray) makes these comparisons fail.

mod common;

use std::collections::BTreeSet;

use bevy::math::Vec3;
use common::*;
use world_core::tree::VoxelSource;
use world_core::{MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_render::internals::batch::UploadBatch;
use world_render::internals::feedback::decode_requests;
use world_render::internals::gbuffer::{HIT_FLAG_HIT, HIT_WORDS};
use world_render::internals::key::{NodeKey, Shape};
use world_render::internals::layout::PoolLayout;
use world_render::internals::residency::Residency;
use world_render::internals::trace::BEAM_OFF;

/// Not a multiple of the 8-pixel tile on either axis.
const SIZE: (u32, u32) = (316, 180);
const BEAM_ITERATIONS: u32 = 64;
/// The step count's word: the only one the prepass may change.
const STEPS_WORD: usize = 6;

fn layout() -> PoolLayout {
    PoolLayout {
        node_count: 1 << 18,
        segments: vec![16 << 20, 16 << 20],
        brick_ids: 1 << 16,
        allocs_per_segment: 1 << 16,
        node_groups: 1 << 16,
        upload_budget: 256 << 20,
        request_capacity: 4096,
        victim_capacity: 1024,
    }
}

/// A floor 32 m across, boxes on it, and thin things that a beam wider
/// than its rays could step over.
fn beam_scene() -> VoxelTree {
    let mut t = VoxelTree::new();
    let fill = |t: &mut VoxelTree, a: [i32; 3], b: [i32; 3], m: u16| {
        t.fill_box(
            VoxelBox::new(VoxelPos::from(a), VoxelPos::from(b)),
            MaterialId(m),
        )
        .unwrap();
    };
    fill(&mut t, [-1024, -16, -1024], [1024, 0, 1024], 1);
    // A low ridge whose top is not aligned to any cell.
    fill(&mut t, [-1024, 0, 300], [1024, 3, 340], 2);
    let mut seed = 0x2545_F491u32;
    let mut next = |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % n) as i32
    };
    // Boxes of mixed sizes.
    for _ in 0..40 {
        let (x, z) = (next(1600) - 800, next(1600) - 800);
        let (w, h, d) = (4 + next(60), 1 + next(90), 4 + next(60));
        fill(&mut t, [x, 0, z], [x + w, h, z + d], 3 + next(3) as u16);
    }
    // Pillars one voxel wide, 100 to 400 voxels away from the cameras.
    for _ in 0..60 {
        let (x, z) = (next(1400) - 700, -(100 + next(700)));
        fill(&mut t, [x, 0, z], [x + 1, 40 + next(260), z + 1], 6);
    }
    // A one-voxel wall along z and a one-voxel plate overhead.
    fill(&mut t, [37, 0, -900], [38, 50, 200], 7);
    fill(&mut t, [-300, 211, -600], [300, 212, -100], 8);
    // Lone voxels in the air.
    for _ in 0..200 {
        let p = VoxelPos::new(next(1200) - 600, 20 + next(300), -(next(900)));
        t.set(p, MaterialId(9)).unwrap();
    }
    t
}

fn views() -> Vec<ViewSetup> {
    vec![
        // Standing, looking down the pillars over the floor.
        ViewSetup {
            eye_m: Vec3::new(0.31, 1.7, 2.13),
            forward: Vec3::new(0.05, -0.12, -1.0),
            vertical_fov_radians: 1.0,
        },
        // A few centimetres above the floor, almost level: every floor ray
        // grazes it for a long way.
        ViewSetup {
            eye_m: Vec3::new(-0.47, 0.09, 1.05),
            forward: Vec3::new(-0.02, -0.004, -1.0),
            vertical_fov_radians: 1.2,
        },
        // Along the thin wall, nearly edge-on.
        ViewSetup {
            eye_m: Vec3::new(0.62, 0.4, 3.3),
            forward: Vec3::new(-0.003, 0.05, -1.0),
            vertical_fov_radians: 0.7,
        },
        // Up at the plate from below, and the sky past it.
        ViewSetup {
            eye_m: Vec3::new(-1.1, 1.2, -1.2),
            forward: Vec3::new(0.2, 0.45, -1.0),
            vertical_fov_radians: 1.1,
        },
        // Down and back across everything, negative on every axis.
        ViewSetup {
            eye_m: Vec3::new(9.0, 7.5, 4.0),
            forward: Vec3::new(-0.8, -0.5, -0.9),
            vertical_fov_radians: 0.9,
        },
    ]
}

fn resident_cache<S: VoxelSource>(
    gpu: &Gpu,
    pipelines: &Pipelines,
    source: &S,
) -> (Residency, GpuCache) {
    let shape = Shape::of(source);
    let layout = layout();
    let mut res = Residency::new(shape, &layout).expect("node space");
    let mut pools = world_render::internals::batch::CpuPools::new(
        layout.node_count,
        &layout.segments,
        layout.brick_ids,
    );
    let mut cache = GpuCache::new(gpu, pipelines, layout);
    for batch in make_resident(&mut res, source, &mut pools) {
        cache.apply(gpu, pipelines, &batch);
    }
    (res, cache)
}

/// The nodes requested and the brick ids stamped with `frame`, after one
/// trace. A request's priority is not compared: it comes from whichever of
/// the node's rays wins the race to request it, which varies from run to
/// run with or without the prepass.
fn side_effects(
    gpu: &Gpu,
    cache: &GpuCache,
    shape: Shape,
    frame: u32,
) -> (BTreeSet<NodeKey>, BTreeSet<usize>) {
    let layout = &cache.pools.layout;
    let feedback = read_words(gpu, &cache.pools.feedback);
    let (requests, stats) = decode_requests(
        &feedback[..layout.request_block_words() as usize],
        layout.request_capacity,
        shape,
    );
    assert_eq!(stats.invalid, 0);
    assert_eq!(
        stats.written as usize,
        requests.len(),
        "requests overflowed"
    );
    let requests = requests.iter().map(|r| r.key).collect();
    let table = read_words(gpu, &cache.pools.table);
    let stamped = table
        .chunks_exact(2)
        .enumerate()
        .filter(|(_, e)| e[1] == frame)
        .map(|(i, _)| i)
        .collect();
    // Clear the request counters for the next trace.
    gpu.queue.write_buffer(&cache.pools.feedback, 0, &[0u8; 8]);
    wait(gpu);
    (requests, stamped)
}

/// Traces every view without and with the prepass and compares them.
fn compare_with_and_without_beam(footprint_bias: f32, evict_every: Option<usize>, label: &str) {
    let Some(gpu) = gpu() else {
        return;
    };
    let pipelines = Pipelines::new(&gpu.device);
    let tree = beam_scene();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let (mut res, mut cache) = resident_cache(&gpu, &pipelines, &snap);
    if let Some(n) = evict_every {
        let mut batch = UploadBatch::default();
        let evicted: Vec<NodeKey> = res.resident_bricks().into_iter().step_by(n).collect();
        assert!(!evicted.is_empty());
        for &k in &evicted {
            res.evict_brick(k, &mut batch).unwrap();
        }
        cache.apply(&gpu, &pipelines, &batch);
    }
    // Clear the request counters the uploads left.
    gpu.queue.write_buffer(&cache.pools.feedback, 0, &[0u8; 8]);

    let (w, h) = SIZE;
    let mut frame = 10;
    let (mut started, mut tiles) = (0usize, 0usize);
    let (mut steps_without, mut steps_with) = (0u64, 0u64);
    let mut requests_seen = 0;
    for (v, view) in views().into_iter().enumerate() {
        let setup = TraceSetup {
            shape,
            frame,
            pixel_angle: view.pixel_angle(h),
            footprint_bias,
        };
        let without = camera_trace(&gpu, &pipelines, &cache, view, SIZE, setup, BEAM_OFF);
        let effects_without = side_effects(&gpu, &cache, shape, frame);
        let setup = TraceSetup {
            frame: frame + 1,
            ..setup
        };
        let with = camera_trace(&gpu, &pipelines, &cache, view, SIZE, setup, BEAM_ITERATIONS);
        let effects_with = side_effects(&gpu, &cache, shape, frame + 1);
        frame += 2;

        assert!(without.beam.iter().all(|&t| t == 0.0));
        let hits = (0..(w * h) as usize)
            .filter(|&i| without.record(i)[3] >> 24 & HIT_FLAG_HIT != 0)
            .count();
        assert!(
            hits > (w * h) as usize / 10,
            "{label} view {v}: only {hits} pixels hit anything"
        );
        for i in 0..(w * h) as usize {
            let (a, b) = (without.record(i), with.record(i));
            steps_without += u64::from(a[STEPS_WORD]);
            steps_with += u64::from(b[STEPS_WORD]);
            let same = (0..HIT_WORDS as usize).all(|k| k == STEPS_WORD || a[k] == b[k]);
            if !same {
                let (x, y) = (i as u32 % w, i as u32 / w);
                let tile = (y / 8 * w.div_ceil(8) + x / 8) as usize;
                panic!(
                    "{label} view {v} pixel ({x}, {y}): {a:?} from the camera, {b:?} from the \
                     beam's start {} on {}",
                    with.beam[tile], gpu.info.name
                );
            }
        }
        assert_eq!(
            effects_without.0, effects_with.0,
            "{label} view {v}: the requests differ"
        );
        assert_eq!(
            effects_without.1, effects_with.1,
            "{label} view {v}: the stamped bricks differ"
        );
        requests_seen += effects_with.0.len();
        tiles += with.beam.len();
        started += with.beam.iter().filter(|&&t| t > 0.0).count();
    }
    eprintln!(
        "{label}: {started} of {tiles} tiles start past the camera; steps {steps_without} \
         without the beam, {steps_with} with it; {requests_seen} requests"
    );
    // The prepass must do something, or the comparison proves nothing.
    assert!(
        started * 2 > tiles,
        "{label}: only {started} of {tiles} tiles moved"
    );
    assert!(
        steps_with < steps_without,
        "{label}: the beam saved no steps"
    );
    if evict_every.is_some() {
        assert!(
            requests_seen > 0,
            "{label}: no ray reached an evicted brick"
        );
    }
}

#[test]
fn gpu_beam_changes_no_hit_at_full_resolution() {
    compare_with_and_without_beam(0.0, None, "full resolution");
}

#[test]
fn gpu_beam_changes_no_hit_with_footprint_detail() {
    compare_with_and_without_beam(1.0, None, "footprint bias 1");
}

#[test]
fn gpu_beam_changes_no_hit_with_coarse_footprints() {
    compare_with_and_without_beam(6.0, None, "footprint bias 6");
}

#[test]
fn gpu_beam_changes_no_hit_or_request_with_evicted_bricks() {
    compare_with_and_without_beam(1.0, Some(3), "every third brick evicted");
}
