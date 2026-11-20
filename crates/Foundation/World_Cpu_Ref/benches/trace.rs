//! Primary rays per second at 256 x 256 on a generated-terrain scene, for the
//! hierarchical reference tracer over a `DenseGrid` and the world
//! `VoxelTree`, against World_Testkit's dense DDA on the same voxels.
//!
//! Run with `cargo bench -p world_cpu_ref --bench trace -- --quick`.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use world_core::{DenseGrid, VoxelPos, VoxelTree};
use world_cpu_ref::camera::{Camera, CameraDesc, Ray};
use world_cpu_ref::trace::{TraceOptions, trace};
use world_gen::{ChunkCoord, Generator, GeneratorVersion, TerrainConfig};
use world_testkit::dense::{Bounds, DenseWorld, VoxelModel};
use world_testkit::hits;

const SIDE: u32 = 256;

/// A 64 x 64 terrain patch (coarse level 2 of a hilly chunk, heights / 16)
/// in a 64-voxel cube, as a grid, a tree and a dense model.
fn scene() -> (DenseGrid, VoxelTree, DenseWorld, i32) {
    let g = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
        .expect("valid default");
    let coarse = g
        .coarse_columns(ChunkCoord::new(576, 512, 280).expect("in the world"), 2)
        .expect("valid level");
    let heights: Vec<i32> = coarse
        .columns()
        .iter()
        .map(|c| c.height.div_euclid(16))
        .collect();
    let low = heights.iter().copied().min().unwrap_or(0) - 8;
    let origin = VoxelPos::new(0, low, 0);
    let mut grid = DenseGrid::new(3, origin);
    let mut tree = VoxelTree::new();
    let mut dense = DenseWorld::new(Bounds::new(origin, [64; 3]));
    let layers = coarse.layers();
    for j in 0..64u32 {
        for i in 0..64u32 {
            let column = coarse.column(i, j).expect("64 samples per axis");
            let top = heights[(j * 64 + i) as usize].min(low + 63);
            for y in low..top {
                let m = column.material_at(column.height - 1 - 16 * (top - 1 - y), &layers);
                let p = VoxelPos::new(i as i32, y, j as i32);
                grid.set(p, m);
                tree.set(p, m).expect("in the world");
                dense.set(p, m);
            }
        }
    }
    (grid, tree, dense, low)
}

fn rays(low: i32) -> Vec<Ray> {
    let camera = Camera::new(CameraDesc {
        eye: [-24.0, (low + 80) as f32, -24.0],
        forward: [1.0, -0.7, 1.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov_y: 0.6,
        width: SIDE,
        height: SIDE,
    })
    .expect("valid camera");
    (0..SIDE)
        .flat_map(|y| (0..SIDE).map(move |x| (x, y)))
        .filter_map(|(x, y)| camera.ray(x, y))
        .collect()
}

fn primary_rays(c: &mut Criterion) {
    let (grid, tree, dense, low) = scene();
    let rays = rays(low);
    let options = TraceOptions::default();
    let mut group = c.benchmark_group("primary_rays_256x256");
    group.sample_size(10);
    group.throughput(Throughput::Elements(rays.len() as u64));
    group.bench_function("reference_dense_grid", |b| {
        b.iter(|| {
            let mut hit = 0u32;
            for ray in &rays {
                hit += u32::from(matches!(trace(&grid, ray, &options), Ok(Some(_))));
            }
            black_box(hit)
        })
    });
    group.bench_function("reference_world_tree", |b| {
        b.iter(|| {
            let mut hit = 0u32;
            for ray in &rays {
                hit += u32::from(matches!(trace(&tree, ray, &options), Ok(Some(_))));
            }
            black_box(hit)
        })
    });
    group.bench_function("reference_coarse_level_2", |b| {
        let coarse = TraceOptions {
            stop_level: 2,
            ..options
        };
        b.iter(|| {
            let mut hit = 0u32;
            for ray in &rays {
                hit += u32::from(matches!(trace(&grid, ray, &coarse), Ok(Some(_))));
            }
            black_box(hit)
        })
    });
    group.bench_function("testkit_dense_dda", |b| {
        b.iter(|| {
            let mut hit = 0u32;
            for ray in &rays {
                let r = hits::Ray {
                    origin: ray.origin,
                    dir: ray.dir,
                };
                hit += u32::from(dense.cast_ray(&r).is_some());
            }
            black_box(hit)
        })
    });
    group.finish();
}

criterion_group!(benches, primary_rays);
criterion_main!(benches);
