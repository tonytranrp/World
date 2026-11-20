//! Memory figures for representative scenes. Criterion cannot measure bytes,
//! so this bench times nothing: it builds each scene from a fixed seed and
//! prints `VoxelTree::stats` and the ratios that matter for the memory budget.
//!
//! Run with `cargo bench -p world_core --bench memory`. Arguments (such as
//! Criterion's flags from `cargo xtask bench`) are ignored.
//!
//! - Terrain: a 512x512 heightmap built from box-filled columns. Reported per
//!   surface voxel, because the tree's cost follows the surface, not the
//!   volume.
//! - Churn: the same terrain after a long session of random play-like edits,
//!   reporting fragmentation as `free_words / words` (recycled words per live
//!   word) and as the share of word storage sitting in recycled blocks.
//! - Dense: a 128^3 block of random materials, the worst case per voxel.

mod common;

use std::time::Instant;

use common::{Terrain, churn, dense_block};
use world_core::tree::TreeStats;
use world_core::{VoxelPos, VoxelTree};

const TERRAIN_WIDTH: u32 = 512;
const CHURN_STEPS: usize = 1_000_000;

fn main() {
    let started = Instant::now();
    let terrain = Terrain::new(TERRAIN_WIDTH, 7);
    let mut tree = terrain.build();
    let surface = terrain.surface_voxels();
    let columns = u64::from(TERRAIN_WIDTH) * u64::from(TERRAIN_WIDTH);
    println!(
        "memory: terrain {TERRAIN_WIDTH}x{TERRAIN_WIDTH}, {surface} surface voxels \
         ({:.2} per column), built in {:.1?}",
        surface as f64 / columns as f64,
        started.elapsed()
    );
    report("terrain", &tree, surface);

    let started = Instant::now();
    churn(&mut tree, &terrain, CHURN_STEPS, 99);
    println!(
        "memory: {CHURN_STEPS} random edits near the surface in {:.1?}",
        started.elapsed()
    );
    report("terrain after churn", &tree, surface);

    let edge = 128;
    let dense = dense_block(VoxelPos::ORIGIN, edge, 8, 11);
    let voxels = u64::from(edge).pow(3);
    report("dense 128^3 (per voxel)", &dense, voxels);
}

/// Prints the stats of `tree` and its cost per `unit` (surface voxels, or
/// voxels for the dense block).
fn report(name: &str, tree: &VoxelTree, unit: u64) {
    let TreeStats {
        branches,
        leaves,
        words,
        free_nodes,
        free_words,
        bytes,
    } = tree.stats();
    let nodes = branches + leaves;
    let per = |n: usize| n as f64 / unit as f64;
    let live_bytes = nodes * 16 + words * 4;
    let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    println!("memory: {name}");
    println!(
        "  bytes held {bytes} ({:.2} MiB), live {live_bytes} ({:.2} MiB, nodes x16 + words x4)",
        bytes as f64 / (1 << 20) as f64,
        live_bytes as f64 / (1 << 20) as f64
    );
    println!(
        "  nodes {nodes} ({branches} branches, {leaves} leaves), live words {words}, \
         free nodes {free_nodes}, free words {free_words}"
    );
    println!(
        "  per unit: {:.3} bytes held, {:.3} live bytes, {:.4} nodes, {:.3} live words",
        per(bytes),
        per(live_bytes),
        per(nodes),
        per(words)
    );
    println!(
        "  fragmentation: free_words / words = {:.4}, free share of word storage {:.2}%",
        ratio(free_words, words),
        ratio(free_words, words + free_words) * 100.0
    );
}
