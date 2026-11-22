//! Reads and edits on real generated chunks, built as the residency workers
//! build them (`build_region` through `Generator::chunk_source`, decision
//! 0006): point lookups near the surface, ray-like cursor walks, a walk over
//! every brick as the GPU encoder makes it, and edit batches. These measure
//! the storage layout of builder-made chunks, which World_Core's own benches
//! (trees made by edits) do not see.
//!
//! Run with `cargo bench -p world_gen --bench chunk_reads`.

use std::hint::black_box;
use std::time::{Duration, Instant};

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use world_core::tree::{CHUNK_LEVEL, NodeRef, NodeView};
use world_core::{Edit, MaterialId, VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkColumns, ChunkCoord, Generator, GeneratorVersion, TerrainConfig};

/// A tiny deterministic generator (xorshift64).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u32
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// The surface chunks (and one below) of a 3x3 footprint, in one tree.
struct Scene {
    tree: VoxelTree,
    footprints: Vec<ChunkColumns>,
    chunks: usize,
}

impl Scene {
    fn new() -> Self {
        let g = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
            .expect("the default config is valid");
        let mut tree = VoxelTree::new();
        let mut footprints = Vec::new();
        let mut chunks = 0;
        for dz in 0..3 {
            for dx in 0..3 {
                let footprint = ChunkCoord::new(516 + dx, 512, 508 + dz).expect("inside the world");
                let columns = g.columns(footprint);
                let (lo, hi) = columns.height_range();
                let min = footprint.min_voxel();
                let bottom = ChunkCoord::containing(VoxelPos::new(min.x, lo, min.z)).unwrap();
                let top = ChunkCoord::containing(VoxelPos::new(min.x, hi, min.z)).unwrap();
                for y in bottom.y() - 1..=top.y() {
                    let at = ChunkCoord::new(footprint.x(), y, footprint.z()).unwrap();
                    tree.build_region(CHUNK_LEVEL, at.min_voxel(), &g.chunk_source(at))
                        .expect("an aligned chunk");
                    chunks += 1;
                }
                footprints.push(columns);
            }
        }
        Self {
            tree,
            footprints,
            chunks,
        }
    }

    /// A random voxel within 16 voxels of the surface.
    fn near_surface(&self, rng: &mut Rng) -> VoxelPos {
        let columns = &self.footprints[rng.below(self.footprints.len() as u32) as usize];
        let (x, z) = (rng.below(CHUNK_SIZE), rng.below(CHUNK_SIZE));
        let height = columns.column(x, z).expect("inside the footprint").height;
        let min = columns.chunk().min_voxel();
        VoxelPos::new(
            min.x + x as i32,
            height - 16 + rng.below(32) as i32,
            min.z + z as i32,
        )
    }
}

fn reads(c: &mut Criterion) {
    let scene = Scene::new();
    let stats = scene.tree.stats();
    eprintln!(
        "chunk_reads: {} chunks, {:.1} MiB held, {} bricks and upper nodes, {} words",
        scene.chunks,
        stats.bytes as f64 / (1 << 20) as f64,
        stats.branches,
        stats.words
    );
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let probes: Vec<VoxelPos> = (0..100_000).map(|_| scene.near_surface(&mut rng)).collect();
    // Rays from 64 voxels above the surface, about 45 degrees down, in
    // fixed point (1/256 voxel), at most one voxel per axis a step.
    let rays: Vec<([i64; 3], [i64; 3])> = (0..256)
        .map(|_| {
            let p = scene.near_surface(&mut rng);
            let start = [p.x, p.y + 64, p.z].map(|v| i64::from(v) << 8);
            let dir = [
                i64::from(rng.below(513)) - 256,
                -256,
                i64::from(rng.below(513)) - 256,
            ];
            (start, dir)
        })
        .collect();

    let mut group = c.benchmark_group("chunk_reads");
    group.throughput(Throughput::Elements(probes.len() as u64));
    group.bench_function("get_100k_near_surface", |b| {
        b.iter(|| {
            probes
                .iter()
                .filter(|&&p| !scene.tree.get(black_box(p)).is_air())
                .count()
        })
    });
    group.bench_function("cursor_100k_near_surface", |b| {
        b.iter(|| {
            let mut cursor = scene.tree.cursor();
            probes
                .iter()
                .filter(|&&p| !cursor.get(black_box(p)).is_air())
                .count()
        })
    });
    group.throughput(Throughput::Elements(rays.len() as u64));
    group.bench_function("cursor_rays_256", |b| {
        b.iter(|| {
            let mut cursor = scene.tree.cursor();
            let mut steps = 0u64;
            for &(start, dir) in &rays {
                let mut p = start;
                for _ in 0..512 {
                    steps += 1;
                    let v = p.map(|c| (c >> 8) as i32);
                    if !cursor.get(VoxelPos::new(v[0], v[1], v[2])).is_air() {
                        break;
                    }
                    for a in 0..3 {
                        p[a] += dir[a];
                    }
                }
            }
            black_box(steps)
        })
    });
    group.throughput(Throughput::Elements(scene.chunks as u64));
    group.bench_function("walk_every_brick", |b| {
        b.iter(|| {
            let mut sum = 0u64;
            walk(scene.tree.root(), &mut sum);
            black_box(sum)
        })
    });
    group.finish();
}

/// Visits every brick as the GPU encoder does: every leaf's mask, and its
/// palette block or one voxel.
fn walk(node: NodeRef<'_>, sum: &mut u64) {
    let NodeRef::Branch(b) = node else {
        return;
    };
    if b.level() > 1 {
        for (_, child) in b.children() {
            walk(child, sum);
        }
        return;
    }
    for (slot, leaf) in b.children() {
        *sum += u64::from(slot);
        match leaf {
            NodeRef::Leaf(l) => {
                *sum += u64::from(l.mask().count_ones());
                match leaf.palette_block() {
                    Some(block) => *sum += u64::from(block[block.len() - 1]),
                    None => *sum += u64::from(leaf.voxel(l.mask().trailing_zeros()).0),
                }
            }
            NodeRef::Uniform(m) => *sum += u64::from(m.0),
            _ => {}
        }
    }
}

/// Edit batches near the surface: a crater (every voxel of a 12-voxel
/// sphere set to air, then restored) and 1,000 scattered voxels (set to
/// stone, then restored). Both halves are timed.
fn edits(c: &mut Criterion) {
    let mut scene = Scene::new();
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    let centre = scene.near_surface(&mut rng);
    let mut crater = Vec::new();
    for z in -12i32..=12 {
        for y in -12i32..=12 {
            for x in -12i32..=12 {
                if x * x + y * y + z * z <= 144 {
                    crater.push(VoxelPos::new(centre.x + x, centre.y + y, centre.z + z));
                }
            }
        }
    }
    let scattered: Vec<VoxelPos> = (0..1_000).map(|_| scene.near_surface(&mut rng)).collect();
    let mut group = c.benchmark_group("chunk_edits");
    for (name, voxels, material) in [
        ("crater_r12_and_restore", &crater, MaterialId::AIR),
        ("scattered_1k_and_restore", &scattered, MaterialId(1)),
    ] {
        let restore: Vec<Edit> = voxels
            .iter()
            .map(|&p| Edit::new(p, scene.tree.get(p)))
            .collect();
        let carve: Vec<Edit> = voxels.iter().map(|&p| Edit::new(p, material)).collect();
        group.throughput(Throughput::Elements(2 * voxels.len() as u64));
        group.bench_function(name, |b| {
            b.iter_batched(
                || (),
                |()| {
                    scene.tree.apply(&carve).unwrap();
                    scene.tree.apply(&restore).unwrap();
                },
                BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

/// Every voxel of the ball of `radius` around `centre`.
fn ball(centre: VoxelPos, radius: i32) -> Vec<VoxelPos> {
    let mut voxels = Vec::new();
    for z in -radius..=radius {
        for y in -radius..=radius {
            for x in -radius..=radius {
                if x * x + y * y + z * z <= radius * radius {
                    voxels.push(VoxelPos::new(centre.x + x, centre.y + y, centre.z + z));
                }
            }
        }
    }
    voxels
}

/// The destruction pattern: craters (a ball of air centred on the surface,
/// one `apply`, radius 16 to 128 voxels, a quarter metre to two metres)
/// and a tunnel walk (24 balls of radius 24, each its own `apply` as a dig
/// per frame, stepping 12 voxels into the ground). Only the carving is
/// timed; the restore that follows each round (one `apply` of the old
/// materials) is not. Throughput counts the voxels the edits name.
fn destruction(c: &mut Criterion) {
    let mut scene = Scene::new();
    let mut rng = Rng(0x0DDB_A11C_AFE5_EED5);
    let surface = |scene: &Scene, rng: &mut Rng| {
        let p = scene.near_surface(rng);
        let columns = scene
            .footprints
            .iter()
            .find(|c| {
                let min = c.chunk().min_voxel();
                (0..CHUNK_SIZE as i32).contains(&(p.x - min.x))
                    && (0..CHUNK_SIZE as i32).contains(&(p.z - min.z))
            })
            .expect("a footprint holds the probe");
        let min = columns.chunk().min_voxel();
        let height = columns
            .column((p.x - min.x) as u32, (p.z - min.z) as u32)
            .expect("inside the footprint")
            .height;
        VoxelPos::new(p.x, height, p.z)
    };
    let mut cases: Vec<(String, Vec<Vec<VoxelPos>>)> = Vec::new();
    for radius in [16, 32, 64, 128] {
        let centre = surface(&scene, &mut rng);
        cases.push((format!("crater_r{radius}"), vec![ball(centre, radius)]));
    }
    let start = surface(&scene, &mut rng);
    let tunnel = (0..24)
        .map(|i| VoxelPos::new(start.x + 12 * i, start.y - 6 * i, start.z + 4 * i))
        .map(|centre| ball(centre, 24))
        .collect();
    cases.push(("tunnel_24_digs_r24".into(), tunnel));

    let mut group = c.benchmark_group("destruction");
    group.sample_size(10);
    for (name, digs) in &cases {
        // Overlapping digs name a voxel twice, with the same old material.
        let restore: Vec<Edit> = digs
            .iter()
            .flatten()
            .map(|&p| Edit::new(p, scene.tree.get(p)))
            .collect();
        let carves: Vec<Vec<Edit>> = digs
            .iter()
            .map(|d| d.iter().map(|&p| Edit::new(p, MaterialId::AIR)).collect())
            .collect();
        let voxels: usize = digs.iter().map(Vec::len).sum();
        group.throughput(Throughput::Elements(voxels as u64));
        group.bench_function(name.as_str(), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let started = Instant::now();
                    for carve in &carves {
                        black_box(scene.tree.apply(carve).unwrap());
                    }
                    total += started.elapsed();
                    scene.tree.apply(&restore).unwrap();
                }
                total
            })
        });
    }
    group.finish();
}

criterion_group!(benches, reads, edits, destruction);
criterion_main!(benches);
