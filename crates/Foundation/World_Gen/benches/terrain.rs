//! Terrain generation throughput: full-resolution columns per chunk, coarse
//! sampling, single-column point queries and per-voxel material lookups.
//!
//! Run with `cargo bench -p world_gen --bench terrain -- --quick`.

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use world_gen::{CHUNK_SIZE, ChunkCoord, Generator, GeneratorVersion, TerrainConfig};

fn generator() -> Generator {
    Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
        .expect("the default config is valid")
}

fn chunk() -> ChunkCoord {
    ChunkCoord::new(517, 512, 509).expect("inside the world")
}

fn columns(c: &mut Criterion) {
    let g = generator();
    let mut group = c.benchmark_group("terrain");
    group.sample_size(10);
    group.throughput(Throughput::Elements(u64::from(CHUNK_SIZE * CHUNK_SIZE)));
    group.bench_function("chunk_columns", |b| {
        b.iter(|| g.columns(black_box(chunk())))
    });
    group.finish();
}

fn coarse(c: &mut Criterion) {
    let g = generator();
    let mut group = c.benchmark_group("terrain_coarse");
    group.sample_size(10);
    for level in [1u32, 2, 3] {
        let samples = u64::from(CHUNK_SIZE >> (2 * level)).pow(2);
        group.throughput(Throughput::Elements(samples));
        group.bench_function(format!("level_{level}"), |b| {
            b.iter(|| g.coarse_columns(black_box(chunk()), level))
        });
    }
    group.finish();
}

fn point_queries(c: &mut Criterion) {
    let g = generator();
    let min = chunk().min_voxel();
    let mut group = c.benchmark_group("terrain_point");
    group.throughput(Throughput::Elements(64));
    group.bench_function("column_x64", |b| {
        b.iter(|| {
            for i in 0..64 {
                black_box(g.column(min.x + i * 13, min.z + i * 7).ok());
            }
        })
    });
    group.finish();
}

fn material_queries(c: &mut Criterion) {
    let g = generator();
    let cols = g.columns(chunk());
    let (lo, hi) = cols.height_range();
    let span = (hi - lo + 64).max(1);
    let mut group = c.benchmark_group("terrain_material");
    let n = 1u64 << 20;
    group.throughput(Throughput::Elements(n));
    group.bench_function("material_per_voxel", |b| {
        b.iter(|| {
            let mut state = 0x1234_5678u32;
            let mut solid = 0u32;
            for _ in 0..n {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let x = state & (CHUNK_SIZE - 1);
                let z = (state >> 10) & (CHUNK_SIZE - 1);
                let y = lo - 32 + (state >> 20) as i32 % span;
                solid += u32::from(cols.material(x, y, z).is_some_and(|m| !m.is_air()));
            }
            black_box(solid)
        })
    });
    group.finish();
}

criterion_group!(benches, columns, coarse, point_queries, material_queries);
criterion_main!(benches);
