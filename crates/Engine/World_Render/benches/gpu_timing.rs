//! GPU timings of the trace and apply passes, on the test adapter
//! (`WORLD_TEST_ADAPTER=gpu` for the real GPU). Prints `SKIPPED` without an
//! adapter, so it never fails a machine without one.
//!
//! Uses timestamp queries when the adapter supports them, otherwise wall
//! time around submit and a blocking poll (which includes submission
//! overhead). Not a Criterion bench: GPU frames need their own clock.
//!
//! - trace: 1280 x 720 camera rays over a fully resident terrain, full
//!   resolution and the default footprint bias.
//! - apply: uploading N bricks through the staging belt and the apply pass.

#[path = "../tests/common/mod.rs"]
mod common;

use std::time::Instant;

use bevy::math::{Mat4, Vec3};
use common::*;
use wgpu::util::DeviceExt;
use world_core::{MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_render::internals::batch::UploadBatch;
use world_render::internals::gbuffer::HIT_WORDS;
use world_render::internals::key::Shape;
use world_render::internals::layout::PoolLayout;
use world_render::internals::residency::Residency;
use world_render::internals::trace::{self, MODE_CAMERA, TraceParams};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;
const RUNS: usize = 20;

/// Hills of 512 x 512 voxels (8 m square) with layered materials.
fn terrain() -> VoxelTree {
    let mut t = VoxelTree::new();
    for z in -256..256 {
        for x in -256..256 {
            let h =
                (10.0 * ((x as f32) * 0.03).sin() + 8.0 * ((z as f32) * 0.04).cos()) as i32 - 40;
            let col = |lo: i32, hi: i32| {
                VoxelBox::new(VoxelPos::new(x, lo, z), VoxelPos::new(x + 1, hi, z + 1))
            };
            t.fill_box(col(-90, h - 4), MaterialId(1)).unwrap();
            t.fill_box(col(h - 4, h - 1), MaterialId(2)).unwrap();
            t.fill_box(col(h - 1, h), MaterialId(3)).unwrap();
        }
    }
    t
}

fn layout() -> PoolLayout {
    PoolLayout {
        node_count: 1 << 20,
        segments: vec![128 << 20, 128 << 20],
        brick_ids: 1 << 18,
        allocs_per_segment: 1 << 18,
        node_groups: 1 << 18,
        upload_budget: 1 << 30,
        request_capacity: 16384,
        victim_capacity: 1024,
    }
}

struct Timer {
    set: Option<wgpu::QuerySet>,
    resolve: wgpu::Buffer,
}

impl Timer {
    fn new(gpu: &Gpu) -> Self {
        let set = gpu.timestamps.then(|| {
            gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("bench timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            })
        });
        let resolve = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bench resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self { set, resolve }
    }

    fn writes(&self) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        self.set
            .as_ref()
            .map(|query_set| wgpu::ComputePassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            })
    }

    /// Resolves the pass's timestamps, if any, into the read-back buffer.
    fn finish(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(set) = &self.set {
            encoder.resolve_query_set(set, 0..2, &self.resolve, 0);
        }
    }

    /// Milliseconds of the pass, from timestamps if present, else `wall_ms`.
    fn read_ms(&self, gpu: &Gpu, wall_ms: f64) -> f64 {
        if self.set.is_none() {
            return wall_ms;
        }
        let w = read_words(gpu, &self.resolve);
        let t0 = u64::from(w[0]) | u64::from(w[1]) << 32;
        let t1 = u64::from(w[2]) | u64::from(w[3]) << 32;
        t1.saturating_sub(t0) as f64 * f64::from(gpu.queue.get_timestamp_period()) / 1e6
    }
}

/// A right-handed view matrix looking along `dir` from `eye`.
fn look_to(eye: Vec3, dir: Vec3, up: Vec3) -> Mat4 {
    let f = dir.normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    Mat4::from_cols(
        bevy::math::Vec4::new(s.x, u.x, -f.x, 0.0),
        bevy::math::Vec4::new(s.y, u.y, -f.y, 0.0),
        bevy::math::Vec4::new(s.z, u.z, -f.z, 0.0),
        bevy::math::Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0),
    )
}

/// Bevy's infinite reverse-Z perspective projection.
fn perspective_infinite_reverse(fov_y: f32, aspect: f32, near: f32) -> Mat4 {
    let f = 1.0 / (fov_y / 2.0).tan();
    Mat4::from_cols(
        bevy::math::Vec4::new(f / aspect, 0.0, 0.0, 0.0),
        bevy::math::Vec4::new(0.0, f, 0.0, 0.0),
        bevy::math::Vec4::new(0.0, 0.0, 0.0, -1.0),
        bevy::math::Vec4::new(0.0, 0.0, near, 0.0),
    )
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    // Criterion-style flags from `cargo bench` are ignored.
    let Some(gpu) = gpu() else {
        println!("gpu_timing: SKIPPED (no adapter)");
        return;
    };
    let clock = if gpu.timestamps {
        "timestamp queries"
    } else {
        "wall time"
    };
    println!(
        "gpu_timing on {} ({:?}), {clock}",
        gpu.info.name, gpu.info.backend
    );
    let pipelines = Pipelines::new(&gpu.device);
    let timer = Timer::new(&gpu);

    // ------------------------------------------------------------ apply --
    let tree = terrain();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let mut res = Residency::new(shape, &layout()).unwrap();
    let mut batches = Vec::new();
    let mut batch = UploadBatch::default();
    res.reset(&snap, &mut batch);
    loop {
        let mut changed = false;
        for k in res.keys() {
            if k.level >= 2 && !res.is_expanded(k) {
                changed |= res.expand(&snap, k, &mut batch).is_ok();
            } else if k.level == 1 && !res.is_brick_resident(k) {
                changed |= res.load_brick(&snap, k, &mut batch).is_ok();
            }
        }
        batches.push(std::mem::take(&mut batch));
        if !changed {
            break;
        }
    }
    let mut cache = GpuCache::new(&gpu, &pipelines, layout());
    for b in &batches {
        cache.apply(&gpu, &pipelines, b);
    }
    let bricks = res.resident_bricks();
    println!(
        "scene: {} resident bricks, {} records",
        bricks.len(),
        res.keys().len()
    );

    for n in [256usize, 2048, bricks.len().min(16384)] {
        // Evict then reload n bricks: one batch of n block uploads.
        let mut times = Vec::new();
        let mut bytes = 0;
        for _ in 0..RUNS.min(8) {
            let mut b = UploadBatch::default();
            for &k in bricks.iter().take(n) {
                let _ = res.evict_brick(k, &mut b);
            }
            cache.apply(&gpu, &pipelines, &b);
            let mut b = UploadBatch::default();
            for &k in bricks.iter().take(n) {
                let _ = res.load_brick(&snap, k, &mut b);
            }
            bytes = b.data_bytes();
            let mut words = Vec::new();
            let counts = b.serialize(&mut words);
            cache.upload.reserve(&gpu.device, words.len() as u64 * 4);
            let layout_bg = world_render::internals::upload::apply_bind_group(
                &gpu.device,
                &pipelines.apply_layout,
                &cache.pools,
                &cache.upload,
            );
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            // Time the apply pass alone: copy first, then a timed pass.
            world_render::internals::upload::record_apply(
                &mut encoder,
                &mut cache.belt,
                &cache.upload,
                &words,
                counts,
                &layout_bg,
                &pipelines.apply_pipelines(),
            );
            cache.belt.finish_and_recall_on_submit(&encoder);
            let start = Instant::now();
            gpu.queue.submit([encoder.finish()]);
            wait(&gpu);
            times.push(start.elapsed().as_secs_f64() * 1e3);
        }
        println!(
            "apply {n:>6} bricks ({:>6} KiB): {:.3} ms median wall (upload copy + 4 dispatches)",
            bytes >> 10,
            median(times)
        );
    }

    // ------------------------------------------------------------ trace --
    let hits = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bench hits"),
        size: u64::from(WIDTH * HEIGHT * HIT_WORDS) * 4,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let eye_m = Vec3::new(0.3, 0.6, 3.5);
    let view = look_to(eye_m, Vec3::new(0.0, -0.35, -1.0).normalize(), Vec3::Y);
    let fov = 70f32.to_radians();
    let proj = perspective_infinite_reverse(fov, WIDTH as f32 / HEIGHT as f32, 0.05);
    let clip_from_world = proj * view;
    let o = shape.origin;
    for (label, bias) in [("full resolution", 0.0f32), ("footprint bias 1", 1.0)] {
        let params = TraceParams {
            clip_from_world: clip_from_world.to_cols_array_2d(),
            world_from_clip: clip_from_world.inverse().to_cols_array_2d(),
            eye: [eye_m.x * 64.0, eye_m.y * 64.0, eye_m.z * 64.0, 0.0],
            origin: [o.x, o.y, o.z, shape.root_level() as i32],
            size: [WIDTH, HEIGHT, 2, MODE_CAMERA],
            lod: [2.0 * (fov / 2.0).tan() / HEIGHT as f32, bias, 64.0, 0.0],
            limits: [
                cache.pools.layout.request_capacity,
                cache.pools.layout.stamps_offset(),
                trace::DEFAULT_MAX_ITERATIONS,
                0,
            ],
        };
        let params_buf = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bench params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = trace::trace_bind_group(
            &gpu.device,
            &pipelines.trace_layout,
            &cache.pools,
            &hits,
            &params_buf,
        );
        let mut times = Vec::new();
        for run in 0..RUNS + 3 {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            trace::record_trace(
                &mut encoder,
                &pipelines.trace,
                &bind_group,
                WIDTH,
                HEIGHT,
                timer.writes(),
            );
            timer.finish(&mut encoder);
            let start = Instant::now();
            gpu.queue.submit([encoder.finish()]);
            wait(&gpu);
            let wall = start.elapsed().as_secs_f64() * 1e3;
            if run >= 3 {
                times.push(timer.read_ms(&gpu, wall));
            }
        }
        println!(
            "trace {WIDTH}x{HEIGHT} ({label}): {:.3} ms median over {RUNS} runs",
            median(times)
        );
    }
}
