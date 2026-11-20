//! CPU hot paths of the brick cache.
//!
//! - `encode`: brick blocks per second and MB per second from a terrain-like
//!   tree. Protects the per-frame upload budget: at 24 MiB per frame the PC
//!   must encode ~24k average bricks inside a few milliseconds.
//! - `alloc`: brick space allocate/free churn and node group churn, the
//!   bookkeeping of every upload and eviction.
//! - `residency`: making a whole scene resident (expand + load), the work
//!   of the first frames and of a fast camera turn.
//!
//! - `load_bricks`: loading every brick of the terrain into pools, with the
//!   encoder on 1 and on 8 threads, the path a frame takes.
//!
//! Deterministic input.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use world_core::tree::{NodeView, VoxelSource};
use world_core::{MaterialId, VoxelBox, VoxelPos, VoxelTree};
use world_render::internals::batch::UploadBatch;
use world_render::internals::format::brick::encode_brick;
use world_render::internals::format::summary::Tally;
use world_render::internals::key::{NodeKey, Shape, view_at};
use world_render::internals::layout::PoolLayout;
use world_render::internals::residency::Residency;
use world_render::internals::{alloc, scene};

/// Rolling terrain, 256 x 256 voxels (4 m square), three materials in
/// layers, with scattered single voxels so leaves have palettes.
fn terrain() -> VoxelTree {
    let mut t = VoxelTree::new();
    for z in 0..256 {
        for x in 0..256 {
            let h = (8.0 * ((x as f32) * 0.07).sin() + 6.0 * ((z as f32) * 0.05).cos()) as i32;
            let column = |lo: i32, hi: i32| {
                VoxelBox::new(VoxelPos::new(x, lo, z), VoxelPos::new(x + 1, hi, z + 1))
            };
            t.fill_box(column(-40, h - 6), MaterialId(1)).unwrap();
            t.fill_box(column(h - 6, h - 1), MaterialId(2)).unwrap();
            t.fill_box(column(h - 1, h), MaterialId(3)).unwrap();
            if (x * 7 + z * 13) % 23 == 0 {
                t.set(VoxelPos::new(x, h - 3, z), MaterialId(4)).unwrap();
            }
        }
    }
    t
}

/// Every level-1 node of `source` that needs a block.
fn bricks<S: VoxelSource>(source: &S, shape: Shape) -> Vec<NodeKey> {
    let mut out = Vec::new();
    let mut stack = vec![shape.root()];
    while let Some(k) = stack.pop() {
        let v = view_at(source, k);
        if v.kind() != world_core::tree::NodeKind::Branch {
            continue;
        }
        if k.level == 1 {
            out.push(k);
        } else {
            stack.extend(v.children().map(|(s, _)| k.child(s)));
        }
    }
    out.sort_unstable();
    out
}

fn encode(c: &mut Criterion) {
    let tree = terrain();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let keys = bricks(&snap, shape);
    let mut words = Vec::new();
    let mut tally = Tally::default();
    for &k in &keys {
        encode_brick(view_at(&snap, k), 0, false, &mut words, &mut tally);
    }
    let bytes = words.len() as u64 * 4;
    eprintln!(
        "encode: {} bricks, {} KiB, {:.0} B per brick",
        keys.len(),
        bytes >> 10,
        bytes as f64 / keys.len() as f64
    );
    let mut group = c.benchmark_group("encode");
    group.throughput(Throughput::Elements(keys.len() as u64));
    group.bench_function("terrain_bricks", |b| {
        b.iter(|| {
            words.clear();
            for &k in &keys {
                encode_brick(view_at(&snap, k), 0, false, &mut words, &mut tally);
            }
            words.len()
        })
    });
    group.throughput(Throughput::Bytes(bytes));
    group.bench_function("terrain_bytes", |b| {
        b.iter(|| {
            words.clear();
            for &k in &keys {
                encode_brick(view_at(&snap, k), 0, false, &mut words, &mut tally);
            }
            words.len()
        })
    });
    group.finish();
}

fn allocators(c: &mut Criterion) {
    let mut group = c.benchmark_group("alloc");
    let sizes = [12u32, 40, 96, 260, 800, 64, 128, 200];
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("brick_churn_10k", |b| {
        let mut space = alloc::BrickSpace::new(&[64 << 20, 64 << 20], 1 << 18);
        let mut live = std::collections::VecDeque::new();
        for i in 0..20_000 {
            live.push_back(space.allocate(sizes[i % sizes.len()] * 4).unwrap());
        }
        let mut i = 0usize;
        b.iter(|| {
            for _ in 0..5_000 {
                if let Some(a) = live.pop_front() {
                    space.free(a);
                }
                live.push_back(space.allocate(sizes[i % sizes.len()] * 4).unwrap());
                i += 1;
            }
        })
    });
    group.bench_function("node_group_churn_10k", |b| {
        let mut space = alloc::NodeSpace::new(1 << 20, 1 << 18).unwrap();
        let mut live = std::collections::VecDeque::new();
        for i in 0..20_000u32 {
            live.push_back(space.allocate(1 + i % 64).unwrap());
        }
        let mut i = 0u32;
        b.iter(|| {
            for _ in 0..5_000 {
                if let Some(a) = live.pop_front() {
                    space.free(a);
                }
                live.push_back(space.allocate(1 + i % 64).unwrap());
                i += 1;
            }
        })
    });
    group.finish();
}

fn residency(c: &mut Criterion) {
    let mut group = c.benchmark_group("residency");
    group.sample_size(10);
    for (name, tree) in [("debug_scene", scene::build()), ("terrain", terrain())] {
        let snap = tree.snapshot();
        let shape = Shape::of(&snap);
        let layout = PoolLayout {
            node_count: 1 << 20,
            segments: vec![256 << 20],
            brick_ids: 1 << 18,
            allocs_per_segment: 1 << 18,
            node_groups: 1 << 18,
            upload_budget: 1 << 30,
            request_capacity: 1024,
            victim_capacity: 1024,
        };
        group.bench_with_input(BenchmarkId::new("make_resident", name), &snap, |b, snap| {
            b.iter(|| {
                let mut res = Residency::new(shape, &layout).unwrap();
                let mut batch = UploadBatch::default();
                res.reset(snap, &mut batch);
                loop {
                    let mut changed = false;
                    for k in res.keys() {
                        if k.level >= 2 && !res.is_expanded(k) {
                            changed |= res.expand(snap, k, &mut batch).is_ok();
                        } else if k.level == 1 && !res.is_brick_resident(k) {
                            changed |= res.load_brick(snap, k, &mut batch).is_ok();
                        }
                    }
                    if !changed {
                        break;
                    }
                }
                batch.data_bytes()
            })
        });
    }
    group.finish();
}

fn load_bricks(c: &mut Criterion) {
    use criterion::BatchSize;
    let tree = terrain();
    let snap = tree.snapshot();
    let shape = Shape::of(&snap);
    let layout = PoolLayout {
        node_count: 1 << 20,
        segments: vec![256 << 20],
        brick_ids: 1 << 18,
        allocs_per_segment: 1 << 18,
        node_groups: 1 << 18,
        upload_budget: 1 << 30,
        request_capacity: 1024,
        victim_capacity: 1024,
    };
    // Every node expanded, no brick loaded: what a frame starts from.
    let expanded = || {
        let mut res = Residency::new(shape, &layout).unwrap();
        let mut batch = UploadBatch::default();
        res.reset(&snap, &mut batch);
        loop {
            let todo: Vec<NodeKey> = res
                .keys()
                .into_iter()
                .filter(|k| k.level >= 2 && !res.is_expanded(*k))
                .collect();
            if todo.is_empty() {
                break;
            }
            for k in todo {
                let _ = res.expand(&snap, k, &mut batch);
            }
        }
        let keys: Vec<NodeKey> = res.keys().into_iter().filter(|k| k.level == 1).collect();
        (res, keys)
    };
    let bytes = {
        let (mut res, keys) = expanded();
        let mut batch = UploadBatch::default();
        res.load_bricks(&snap, &keys, 1, |_| true, &mut batch);
        batch.data_bytes()
    };
    let mut group = c.benchmark_group("load_bricks");
    group.sample_size(10);
    group.throughput(Throughput::Bytes(bytes));
    for threads in [1usize, 8] {
        group.bench_function(format!("terrain_{threads}_threads"), |b| {
            b.iter_batched(
                expanded,
                |(mut res, keys)| {
                    let mut batch = UploadBatch::default();
                    res.load_bricks(&snap, &keys, threads, |_| true, &mut batch);
                    batch.data_bytes()
                },
                BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn config() -> Criterion {
    Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(2))
}

criterion_group! {
    name = benches;
    config = config();
    targets = encode, allocators, residency, load_bricks
}
criterion_main!(benches);
