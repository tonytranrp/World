//! The production 64-tree on the same workloads as the dense baseline, so the
//! two can be read side by side: single writes, batches, box fills, reads
//! (scattered and coherent, direct and through a cursor) and snapshots.
//! Inputs come from a fixed seed; setup is outside the timed region.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use world_core::{Edit, MaterialId, VoxelBox, VoxelPos, VoxelTree};

/// Edge of the edited cube, centred on the origin so every level's node
/// boundary runs through it.
const SIZE: u32 = 256;

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

    fn coord(&mut self) -> i32 {
        (self.next() % SIZE) as i32 - SIZE as i32 / 2
    }

    fn pos(&mut self) -> VoxelPos {
        VoxelPos::new(self.coord(), self.coord(), self.coord())
    }
}

fn edits(count: usize, seed: u64) -> Vec<Edit> {
    let mut rng = Lcg(seed);
    (0..count)
        .map(|_| Edit::new(rng.pos(), MaterialId((rng.next() % 8) as u16)))
        .collect()
}

/// Random boxes up to 32 voxels per axis, a quarter of them carving air.
fn boxes(count: usize) -> Vec<(VoxelBox, MaterialId)> {
    let mut rng = Lcg(3);
    (0..count)
        .map(|_| {
            let size = [0; 3].map(|_| 1 + rng.next() % 32);
            (
                VoxelBox::from_min_size(rng.pos(), size),
                MaterialId((rng.next() % 4) as u16),
            )
        })
        .collect()
}

fn filled_tree() -> VoxelTree {
    let mut tree = VoxelTree::new();
    for e in edits(20_000, 1) {
        tree.set(e.pos, e.material).unwrap();
    }
    tree
}

fn set(c: &mut Criterion) {
    let list = edits(10_000, 1);
    c.bench_function("tree_set_10k_voxels", |b| {
        b.iter_batched(
            VoxelTree::new,
            |mut tree| {
                for e in &list {
                    tree.set(e.pos, e.material).unwrap();
                }
                tree
            },
            BatchSize::LargeInput,
        )
    });
}

/// The same 10,000 edits as `tree_set_10k_voxels`, as one batch.
fn apply(c: &mut Criterion) {
    let list = edits(10_000, 1);
    c.bench_function("tree_apply_10k_voxels", |b| {
        b.iter_batched(
            VoxelTree::new,
            |mut tree| {
                tree.apply(&list).unwrap();
                tree
            },
            BatchSize::LargeInput,
        )
    });
}

fn fill_box(c: &mut Criterion) {
    let list = boxes(64);
    c.bench_function("tree_fill_64_boxes", |b| {
        b.iter_batched(
            VoxelTree::new,
            |mut tree| {
                for &(region, material) in &list {
                    tree.fill_box(region, material).unwrap();
                }
                tree
            },
            BatchSize::LargeInput,
        )
    });
}

fn get(c: &mut Criterion) {
    let tree = filled_tree();
    let mut rng = Lcg(5);
    let probes: Vec<VoxelPos> = (0..100_000).map(|_| rng.pos()).collect();
    c.bench_function("tree_get_100k_voxels", |b| {
        b.iter(|| probes.iter().filter(|&&p| !tree.get(p).is_air()).count())
    });
    c.bench_function("tree_cursor_100k_voxels", |b| {
        b.iter(|| {
            let mut cursor = tree.cursor();
            probes.iter().filter(|&&p| !cursor.get(p).is_air()).count()
        })
    });
}

/// Every voxel of a 40-voxel cube at the origin, x fastest: the access
/// pattern of region scans, neighbour reads and short rays.
fn coherent(c: &mut Criterion) {
    let tree = dense_tree();
    let scan: Vec<VoxelPos> = (0..40)
        .flat_map(|z| (0..40).flat_map(move |y| (0..40).map(move |x| VoxelPos::new(x, y, z))))
        .collect();
    c.bench_function("tree_get_coherent_64k_voxels", |b| {
        b.iter(|| scan.iter().filter(|&&p| !tree.get(p).is_air()).count())
    });
    c.bench_function("tree_cursor_coherent_64k_voxels", |b| {
        b.iter(|| {
            let mut cursor = tree.cursor();
            scan.iter().filter(|&&p| !cursor.get(p).is_air()).count()
        })
    });
}

/// Terrain-like content around the origin: a solid floor with a rough,
/// mixed-material surface, so a scan crosses uniform, single-material and
/// palette leaves.
fn dense_tree() -> VoxelTree {
    let mut tree = VoxelTree::new();
    tree.fill_box(
        VoxelBox::new(VoxelPos::new(-64, -64, -64), VoxelPos::new(64, 8, 64)),
        MaterialId(1),
    )
    .unwrap();
    let mut rng = Lcg(11);
    let surface: Vec<Edit> = (0..20_000)
        .map(|_| {
            let (x, z) = (rng.coord() / 2, rng.coord() / 2);
            let y = 8 + (rng.next() % 12) as i32;
            Edit::new(VoxelPos::new(x, y, z), MaterialId((rng.next() % 4) as u16))
        })
        .collect();
    tree.apply(&surface).unwrap();
    tree
}

fn snapshot(c: &mut Criterion) {
    let tree = filled_tree();
    c.bench_function("tree_snapshot", |b| b.iter(|| black_box(tree.snapshot())));
}

criterion_group!(benches, set, apply, fill_box, get, coherent, snapshot);
criterion_main!(benches);
