//! Writes valid encodings of small representative trees into every directory
//! named on the command line, as the fuzzers' starting corpus:
//! `make_seeds corpus/decode corpus/round_trip`.
//!
//! The trees cover every child tag and every leaf index width (0, 1, 2, 4
//! and 8 bits), the world's corners and a uniform root. Inputs come from a
//! fixed seed, so the files are the same on every run.

use std::fs;
use std::path::PathBuf;

use world_core::serialize::encode;
use world_core::{MaterialId, VoxelBox, VoxelPos, VoxelTree};

struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u32) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as u32) % n
    }
}

/// A cube of `edge` voxels at `min` with random materials from `1..=count`.
fn mixed(min: VoxelPos, edge: i32, count: u32, seed: u64) -> VoxelTree {
    let mut rng = Lcg(seed);
    let mut tree = VoxelTree::new();
    for z in 0..edge {
        for y in 0..edge {
            for x in 0..edge {
                let m = MaterialId(1 + rng.below(count) as u16);
                tree.set(VoxelPos::new(min.x + x, min.y + y, min.z + z), m)
                    .unwrap();
            }
        }
    }
    tree
}

fn seeds() -> Vec<(&'static str, VoxelTree)> {
    let mut seeds = vec![("empty", VoxelTree::new())];

    let mut world = VoxelTree::new();
    world.fill_box(VoxelBox::WORLD, MaterialId(7)).unwrap();
    seeds.push(("uniform_world", world));

    let edge = VoxelBox::WORLD.end();
    for (name, pos) in [
        ("voxel_origin", VoxelPos::ORIGIN),
        ("voxel_min_corner", VoxelBox::WORLD.min()),
        (
            "voxel_max_corner",
            VoxelPos::new(edge.x - 1, edge.y - 1, edge.z - 1),
        ),
    ] {
        let mut tree = VoxelTree::new();
        tree.set(pos, MaterialId(3)).unwrap();
        seeds.push((name, tree));
    }

    let mut boxed = VoxelTree::new();
    boxed
        .fill_box(
            VoxelBox::from_min_size(VoxelPos::new(-3, 5, -11), [5, 6, 7]),
            MaterialId(2),
        )
        .unwrap();
    seeds.push(("box", boxed));

    // Two, three, five and seventeen materials give 1, 2, 4 and 8-bit indices.
    for (name, count) in [
        ("mixed_2", 2),
        ("mixed_3", 3),
        ("mixed_5", 5),
        ("mixed_17", 17),
    ] {
        seeds.push((name, mixed(VoxelPos::new(-4, -4, -4), 8, count, u64::from(count))));
    }

    // Solid ground with a crater: uniform children next to split ones.
    let mut carved = VoxelTree::new();
    carved
        .fill_box(VoxelBox::from_min_size(VoxelPos::ORIGIN, [64; 3]), MaterialId(1))
        .unwrap();
    for z in -6i32..=6 {
        for y in -6i32..=6 {
            for x in -6i32..=6 {
                if x * x + y * y + z * z <= 36 {
                    carved
                        .set(VoxelPos::new(30 + x, 63 + y, 30 + z), MaterialId::AIR)
                        .unwrap();
                }
            }
        }
    }
    seeds.push(("crater", carved));

    // Columns of random height: a tiny terrain.
    let mut rng = Lcg(9);
    let mut terrain = VoxelTree::new();
    for z in 0..24 {
        for x in 0..24 {
            let h = rng.below(12) as i32;
            terrain
                .fill_box(
                    VoxelBox::new(VoxelPos::new(x, -16, z), VoxelPos::new(x + 1, h, z + 1)),
                    MaterialId(1 + (h % 3) as u16),
                )
                .unwrap();
        }
    }
    seeds.push(("terrain", terrain));
    seeds
}

fn main() {
    let dirs: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if dirs.is_empty() {
        eprintln!("usage: make_seeds <corpus dir>...");
        std::process::exit(2);
    }
    let seeds: Vec<(&str, Vec<u8>)> = seeds()
        .into_iter()
        .map(|(name, tree)| (name, encode(&tree)))
        .collect();
    for dir in &dirs {
        fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for (name, bytes) in &seeds {
            let path = dir.join(format!("seed_{name}"));
            fs::write(&path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
        println!("{}: {} seeds", dir.display(), seeds.len());
    }
}
