//! The CPU-vs-GPU comparison pipeline, run with the dense model on both sides
//! until the real reference tracer and GPU tracer exist.

use world_core::{MaterialId, VoxelPos};
use world_testkit::dense::{Bounds, DenseWorld, VoxelModel};
use world_testkit::hits::{Camera, HitMismatch, compare_hits, trace, trace_rays};

fn scene() -> DenseWorld {
    let mut w = DenseWorld::new(Bounds::new(VoxelPos::new(0, 0, 0), [16, 16, 16]));
    for x in 4..12 {
        for y in 4..12 {
            w.set(
                VoxelPos::new(x, y, 10),
                MaterialId(1 + (x as u16 + y as u16) % 3),
            );
        }
    }
    w.set(VoxelPos::new(8, 8, 6), MaterialId(5));
    w
}

fn camera() -> Camera {
    Camera {
        eye: [8.3, 8.6, -4.0],
        forward: [0.0, 0.0, 1.0],
        up: [0.0, 1.0, 0.0],
        vertical_fov_radians: 0.9,
    }
}

#[test]
fn a_scene_traced_twice_matches_exactly() {
    let w = scene();
    let rays = camera().rays(24, 18);
    let cpu = trace_rays(&w, &rays);
    let gpu_standin = trace_rays(&w.clone(), &rays);
    assert_eq!(compare_hits(&cpu, &gpu_standin, 0.0), Ok(()));
}

#[test]
fn the_scene_has_hits_and_misses() {
    let records = trace(&scene(), &camera(), 24, 18);
    let hits = records.iter().filter(|r| r.hit.is_some()).count();
    assert!(
        hits > 0 && hits < records.len(),
        "{hits} hits of {}",
        records.len()
    );
}

#[test]
fn a_corrupted_gpu_result_is_caught_and_located() {
    let w = scene();
    let cpu = trace(&w, &camera(), 24, 18);
    let mut gpu = cpu.clone();
    let victim = gpu.iter().position(|r| r.hit.is_some()).unwrap();
    gpu[victim].hit.as_mut().unwrap().material = MaterialId(99);
    match compare_hits(&cpu, &gpu, 0.0).unwrap_err() {
        HitMismatch::PixelDiffers { index, .. } => assert_eq!(index, victim),
        other => panic!("unexpected {other:?}"),
    }
}
