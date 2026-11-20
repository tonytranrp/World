//! Realistic wall-time workloads for the 64-tree, beyond the microbenchmarks
//! in `tree.rs`: overwriting an already dense region, carving air through
//! solid ground, coherent reads (a line and a whole brick), `apply` batches,
//! the copy-on-write cost of the first edit after a snapshot of a large tree,
//! and serialization throughput.
//!
//! Inputs come from fixed seeds (`common`); scene construction is outside the
//! timed regions. Run with `cargo bench -p world_core --bench workloads`.

mod common;

use std::hint::black_box;
use std::time::{Duration, Instant};

use common::{Lcg, Terrain, dense_block, random_edits, solid_block, sphere, striped_block};
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use world_core::serialize::{decode, encode};
use world_core::{Edit, MaterialId, VoxelBox, VoxelPos, VoxelTree};

/// Edge of the dense random block: 2 Mi voxels, every leaf full and mixed.
const DENSE_EDGE: u32 = 128;
/// Materials in the dense block; 8 gives 3-bit palettes, packed as 4 bits.
const DENSE_MATERIALS: u16 = 8;
/// Width of the terrain used for reads and serialization.
const TERRAIN_WIDTH: u32 = 512;
/// Edge of the striped cube used for snapshots: 1 Gi voxels, a tree of
/// several hundred MB (the exact size is printed when the bench starts).
const LARGE_EDGE: u32 = 1024;

fn dense() -> VoxelTree {
    dense_block(VoxelPos::ORIGIN, DENSE_EDGE, DENSE_MATERIALS, 11)
}

/// Overwrites random voxels inside a region that is already full of mixed
/// materials: palette changes and index repacking, never structural growth.
/// The tree persists across iterations. Each iteration rotates every edit's
/// material by one, so every write changes its voxel, while the content stays
/// statistically the same.
fn overwrite_dense(c: &mut Criterion) {
    let mut tree = dense();
    let edits = random_edits(10_000, VoxelPos::ORIGIN, DENSE_EDGE, DENSE_MATERIALS, 12);
    let mut round = 0u16;
    let mut group = c.benchmark_group("overwrite_dense");
    group.throughput(Throughput::Elements(edits.len() as u64));
    group.bench_function("set_10k", |b| {
        b.iter(|| {
            round = round.wrapping_add(1);
            for e in &edits {
                tree.set(e.pos, rotate(e.material, round)).unwrap();
            }
        })
    });
    group.finish();
}

/// `material` moved `by` places along `1..=DENSE_MATERIALS`.
fn rotate(material: MaterialId, by: u16) -> MaterialId {
    MaterialId(1 + (material.0 - 1 + by % DENSE_MATERIALS) % DENSE_MATERIALS)
}

/// Carves air through solid ground: every write splits a uniform child, so
/// this measures node creation and canonical collapse rather than overwrites.
fn carve(c: &mut Criterion) {
    let ground = || solid_block(VoxelPos::ORIGIN, 256, MaterialId(1));
    // Off the node grid, like an explosion at a random spot.
    let crater = sphere(VoxelPos::new(101, 77, 133), 16);
    // A tunnel of 8x8x8 boxes stepping diagonally through the block.
    let tunnel: Vec<VoxelBox> = (0..32)
        .map(|i| VoxelBox::from_min_size(VoxelPos::new(5 + i * 7, 9 + i * 5, 3 + i * 6), [8; 3]))
        .collect();

    let mut group = c.benchmark_group("carve_solid");
    group.throughput(Throughput::Elements(crater.len() as u64));
    group.bench_function("sphere_r16_set_air", |b| {
        b.iter_batched(
            ground,
            |mut tree| {
                for &pos in &crater {
                    tree.set(pos, MaterialId::AIR).unwrap();
                }
                tree
            },
            BatchSize::LargeInput,
        )
    });
    group.throughput(Throughput::Elements(tunnel.len() as u64));
    group.bench_function("tunnel_32_boxes_fill_air", |b| {
        b.iter_batched(
            ground,
            |mut tree| {
                for &region in &tunnel {
                    tree.fill_box(region, MaterialId::AIR).unwrap();
                }
                tree
            },
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

/// Coherent reads, as a ray march or a brick upload would do them: a line of
/// consecutive voxels, and every voxel of one 16^3 brick in x-fastest order.
fn coherent_reads(c: &mut Criterion) {
    let terrain = Terrain::new(TERRAIN_WIDTH, 7);
    let ground = terrain.build();
    let dense = dense();

    // A horizontal line that crosses the surface many times.
    let line: Vec<VoxelPos> = (0..TERRAIN_WIDTH as i32)
        .map(|x| VoxelPos::new(x, 30, 200))
        .collect();
    // A brick that contains the surface, and one inside the dense block.
    let brick = |min: VoxelPos| -> Vec<VoxelPos> {
        let mut voxels = Vec::with_capacity(4096);
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    voxels.push(VoxelPos::new(min.x + x, min.y + y, min.z + z));
                }
            }
        }
        voxels
    };
    let (bx, bz) = (256, 256);
    let surface_brick = brick(VoxelPos::new(bx, terrain.height(bx, bz) & !15, bz));
    let dense_brick = brick(VoxelPos::new(48, 48, 48));

    let mut group = c.benchmark_group("coherent_reads");
    let mut read = |name: &str, tree: &VoxelTree, voxels: &[VoxelPos]| {
        group.throughput(Throughput::Elements(voxels.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                voxels
                    .iter()
                    .filter(|&&p| !tree.get(black_box(p)).is_air())
                    .count()
            })
        });
    };
    read("terrain_line_512", &ground, &line);
    read("terrain_surface_brick_16", &ground, &surface_brick);
    read("dense_brick_16", &dense, &dense_brick);
    group.finish();
}

/// `apply` batches of 1,000 edits on the dense block: clustered in a 16^3
/// region like an explosion's debris, and scattered over the whole block.
/// As in `overwrite_dense`, each batch rotates the materials of the previous
/// one so that it changes what it touches; the rotated copy is made outside
/// the timed region.
fn apply_batches(c: &mut Criterion) {
    let mut tree = dense();
    let clustered = random_edits(1_000, VoxelPos::new(40, 40, 40), 16, DENSE_MATERIALS, 21);
    let scattered = random_edits(1_000, VoxelPos::ORIGIN, DENSE_EDGE, DENSE_MATERIALS, 22);
    let mut group = c.benchmark_group("apply_1k");
    group.throughput(Throughput::Elements(1_000));
    for (name, edits) in [("clustered_16", &clustered), ("scattered_128", &scattered)] {
        let mut round = 0u16;
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    round = round.wrapping_add(1);
                    edits
                        .iter()
                        .map(|e| Edit::new(e.pos, rotate(e.material, round)))
                        .collect::<Vec<_>>()
                },
                |batch| tree.apply(&batch).unwrap(),
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

/// The first edit after a snapshot pays for copy-on-write: every page on the
/// edit's path, plus whatever bookkeeping is shared, is copied because the
/// snapshot still holds it. `tree_snapshot` in `tree.rs` measures only the
/// O(1) snapshot itself; this measures what it defers. The snapshot is taken
/// and dropped outside the timed region.
fn snapshot_then_edit(c: &mut Criterion) {
    let started = Instant::now();
    let mut tree = striped_block(LARGE_EDGE);
    let stats = tree.stats();
    eprintln!(
        "snapshot_then_edit: {LARGE_EDGE}^3 striped tree, {:.1} MiB, {} branches, {} leaves, built in {:.1?}",
        stats.bytes as f64 / (1 << 20) as f64,
        stats.branches,
        stats.leaves,
        started.elapsed()
    );
    let mut rng = Lcg(31);
    let targets: Vec<VoxelPos> = (0..4096)
        .map(|_| rng.pos_in(VoxelPos::ORIGIN, LARGE_EDGE))
        .collect();
    let scattered = random_edits(1_000, VoxelPos::ORIGIN, LARGE_EDGE, 2, 32);

    let mut group = c.benchmark_group("snapshot_then_edit");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));

    // Every edit must change something: cycle through the targets, flipping
    // between two materials that do not occur in the tree on each pass.
    let mut round = 0usize;
    let mut next_edit = move || {
        round += 1;
        let pos = targets[round % targets.len()];
        let pass = round / targets.len();
        (pos, MaterialId(3 + (pass % 2) as u16))
    };

    group.bench_function("no_snapshot_1_edit", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (pos, material) = next_edit();
                let start = Instant::now();
                tree.set(pos, material).unwrap();
                total += start.elapsed();
            }
            total
        })
    });
    group.bench_function("snapshot_1_edit", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (pos, material) = next_edit();
                let snapshot = tree.snapshot();
                let start = Instant::now();
                tree.set(pos, material).unwrap();
                total += start.elapsed();
                drop(black_box(snapshot));
            }
            total
        })
    });
    group.throughput(Throughput::Elements(scattered.len() as u64));
    group.bench_function("snapshot_1k_scattered_edits", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for i in 0..iters {
                let snapshot = tree.snapshot();
                // Alternate materials so every round changes every voxel.
                let material = MaterialId(3 + (i % 2) as u16);
                let start = Instant::now();
                for e in &scattered {
                    tree.set(e.pos, material).unwrap();
                }
                total += start.elapsed();
                drop(black_box(snapshot));
            }
            total
        })
    });
    group.finish();
}

/// Encoding and decoding throughput, in bytes of the encoded form, for the
/// terrain (a realistic save) and the dense block (the worst case per voxel).
fn serialization(c: &mut Criterion) {
    let terrain = Terrain::new(TERRAIN_WIDTH, 7).build();
    let dense = dense();
    let mut group = c.benchmark_group("serialize");
    group.sample_size(20);
    for (name, tree) in [("terrain_512", &terrain), ("dense_128", &dense)] {
        let bytes = encode(tree);
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_function(format!("encode_{name}"), |b| {
            b.iter(|| encode(black_box(tree)))
        });
        group.bench_function(format!("decode_{name}"), |b| {
            b.iter(|| decode(black_box(&bytes)).unwrap())
        });
    }
    group.finish();
}

/// Whole-chunk operations (decision 0006): moving a chunk between trees,
/// compacting it, and compressing its encoding for the warm tier (LZ4) and
/// region files (zstd).
fn chunk_operations(c: &mut Criterion) {
    use world_core::serialize::codec::{self, Codec};
    use world_core::tree::CHUNK_LEVEL;
    let dense = dense();
    let chunk_min = VoxelPos::ORIGIN;
    let mut group = c.benchmark_group("chunks");
    group.sample_size(20);
    group.bench_function("take_and_insert_dense_chunk", |b| {
        b.iter_batched(
            || (dense_clone(&dense), VoxelTree::unloaded()),
            |(mut from, mut to)| {
                let data = from.take_chunk(chunk_min).unwrap();
                to.insert_chunk(chunk_min, data).unwrap();
                (from, to)
            },
            BatchSize::LargeInput,
        )
    });
    let mut fragmented = dense_clone(&dense);
    for e in random_edits(20_000, VoxelPos::ORIGIN, DENSE_EDGE, 1, 99) {
        fragmented.set(e.pos, e.material).unwrap();
    }
    group.bench_function("compact_fragmented_dense_chunk", |b| {
        b.iter_batched(
            || dense_clone(&fragmented),
            |mut t| {
                t.compact_chunk(chunk_min).unwrap();
                t
            },
            BatchSize::LargeInput,
        )
    });
    let bytes = encode(&dense);
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    for codec_kind in [Codec::Lz4, Codec::Zstd] {
        let packed = codec::compress(codec_kind, &bytes);
        eprintln!(
            "chunks: dense_128 encoding {} bytes, {codec_kind:?} {} bytes ({:.1}x)",
            bytes.len(),
            packed.len(),
            bytes.len() as f64 / packed.len() as f64
        );
        group.bench_function(format!("compress_{codec_kind:?}"), |b| {
            b.iter(|| codec::compress(codec_kind, black_box(&bytes)))
        });
        group.bench_function(format!("decompress_{codec_kind:?}"), |b| {
            b.iter(|| codec::decompress(codec_kind, black_box(&packed), bytes.len()).unwrap())
        });
    }
    let _ = CHUNK_LEVEL;
    group.finish();
}

/// A fresh copy of a tree, through its encoding (trees are not `Clone`).
fn dense_clone(tree: &VoxelTree) -> VoxelTree {
    decode(&encode(tree)).unwrap()
}

criterion_group!(
    benches,
    overwrite_dense,
    carve,
    coherent_reads,
    apply_batches,
    snapshot_then_edit,
    serialization,
    chunk_operations
);
criterion_main!(benches);
