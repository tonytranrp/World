//! CPU reference versus GPU tracer, compared hit for hit.
//!
//! The GPU tracer does not exist yet, so the "GPU" side here is a stand-in that
//! wraps the same dense model. When the real tracer lands, only `GpuStandIn`
//! changes; the scene, camera, comparison and golden hash stay.

use world_core::VoxelPos;
use world_testkit::dense::DenseWorld;
use world_testkit::golden::assert_text_golden;
use world_testkit::hash::StateHasher;
use world_testkit::hits::{
    Camera, Hit, HitRecord, HitSource, Ray, compare_hits, trace, trace_rays,
};
use world_tests::{golden, load_scene, scene_path};

/// Replace with a read-back of the GPU primary-ray pass.
struct GpuStandIn(DenseWorld);

impl HitSource for GpuStandIn {
    fn cast(&self, ray: &Ray) -> Option<Hit> {
        self.0.cast(ray)
    }
}

const WIDTH: u32 = 32;
const HEIGHT: u32 = 24;

fn camera() -> Camera {
    Camera {
        eye: [7.7, 8.4, -3.0],
        forward: [0.02, 0.0, 1.0],
        up: [0.0, 1.0, 0.0],
        vertical_fov_radians: 0.8,
    }
}

fn hit_hash(records: &[HitRecord]) -> u64 {
    let mut h = StateHasher::new();
    for r in records {
        h.write_u32(r.pixel.0).write_u32(r.pixel.1);
        match r.hit {
            None => {
                h.write_u8(0);
            }
            Some(hit) => {
                h.write_u8(1)
                    .write_i32(hit.pos.x)
                    .write_i32(hit.pos.y)
                    .write_i32(hit.pos.z)
                    .write_u16(hit.material.0)
                    .write_u8(hit.face as u8)
                    .write_f32(hit.t);
            }
        }
    }
    h.finish()
}

#[test]
fn the_stand_in_gpu_matches_the_cpu_reference_exactly() {
    let scene = load_scene("wall.scene");
    // One set of rays for both tracers, so only the tracers can differ.
    let rays = camera().rays(WIDTH, HEIGHT);
    let cpu = trace_rays(&scene, &rays);
    let gpu = trace_rays(&GpuStandIn(scene.clone()), &rays);
    assert_eq!(compare_hits(&cpu, &gpu, 0.0), Ok(()));
}

#[test]
fn the_scene_exercises_hits_misses_and_several_materials() {
    let records = trace(&load_scene("wall.scene"), &camera(), WIDTH, HEIGHT);
    let hits: Vec<_> = records.iter().filter_map(|r| r.hit).collect();
    assert!(!hits.is_empty() && hits.len() < records.len());
    let materials: std::collections::BTreeSet<_> = hits.iter().map(|h| h.material.0).collect();
    assert!(materials.len() >= 3, "only saw materials {materials:?}");
    assert!(hits.iter().any(|h| h.pos == VoxelPos::new(8, 8, 5)));
}

#[test]
fn the_cpu_reference_image_has_not_drifted() {
    let records = trace(&load_scene("wall.scene"), &camera(), WIDTH, HEIGHT);
    let text = format!(
        "scene wall.scene {WIDTH}x{HEIGHT}\nhit hash {:016x}\n",
        hit_hash(&records)
    );
    assert!(scene_path("wall.scene").exists());
    assert_text_golden(&golden("wall.hits.txt"), &text);
}
