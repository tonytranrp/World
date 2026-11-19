//! Baseline workloads for the reference model. The performance gate compares
//! these against a saved baseline; the 64-tree gets its own benches later.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use world_core::{Edit, MaterialId, VoxelPos};
use world_testkit::dense::{Bounds, DenseWorld, VoxelModel};
use world_testkit::hits::Ray;

const SIZE: u32 = 64;

/// A tiny deterministic generator so benchmark inputs never change.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn unit(&mut self) -> f32 {
        self.next() as f32 / (1u32 << 31) as f32
    }
}

fn bounds() -> Bounds {
    Bounds::new(VoxelPos::ORIGIN, [SIZE; 3])
}

fn edits(count: usize) -> Vec<Edit> {
    let mut rng = Lcg(1);
    (0..count)
        .map(|_| {
            let pos = VoxelPos::new(
                (rng.next() % SIZE) as i32,
                (rng.next() % SIZE) as i32,
                (rng.next() % SIZE) as i32,
            );
            Edit::new(pos, MaterialId((rng.next() % 8) as u16))
        })
        .collect()
}

fn apply_edits(c: &mut Criterion) {
    let list = edits(10_000);
    c.bench_function("dense_apply_10k_edits", |b| {
        b.iter(|| {
            let mut world = DenseWorld::new(bounds());
            for e in &list {
                world.apply(*e);
            }
            black_box(world.solid_count())
        })
    });
}

fn cast_rays(c: &mut Criterion) {
    let mut world = DenseWorld::new(bounds());
    for e in edits(20_000) {
        world.apply(e);
    }
    let mut rng = Lcg(7);
    let rays: Vec<Ray> = (0..1_000)
        .map(|_| Ray {
            origin: [rng.unit() * 64.0, rng.unit() * 64.0, -5.0],
            dir: [rng.unit() - 0.5, rng.unit() - 0.5, 1.0],
        })
        .collect();
    c.bench_function("dense_cast_1k_rays", |b| {
        b.iter(|| rays.iter().filter(|r| world.cast_ray(r).is_some()).count())
    });
}

criterion_group!(benches, apply_edits, cast_rays);
criterion_main!(benches);
