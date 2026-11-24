//! Memory figures for representative scenes. Criterion cannot measure bytes,
//! so this bench times nothing: it builds each scene from a fixed seed and
//! prints `VoxelTree::stats`, `memory_breakdown` and the ratios that matter
//! for the memory budget.
//!
//! Run with `cargo bench -p world_core --bench memory`. Arguments (such as
//! Criterion's flags from `cargo xtask bench`) are ignored.
//!
//! - Terrain: a 512x512 heightmap built from box-filled columns. Reported per
//!   surface voxel, because the tree's cost follows the surface, not the
//!   volume.
//! - Terrain built bottom-up: the same content through a `RegionSource`, as
//!   World_Gen's chunks are built: the storage layout of real chunks.
//! - Churn: the box-filled terrain after a long session of random play-like
//!   edits, reporting fragmentation as `free_words / words` (recycled words
//!   per live word) and as the share of word storage sitting in recycled
//!   blocks.
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
    let bottom_up = terrain.build_bottom_up();
    println!(
        "memory: terrain built bottom-up in {:.1?}",
        started.elapsed()
    );
    report("terrain built bottom-up", &bottom_up, surface);

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
        chunks,
    } = tree.stats();
    // Leaves are not nodes: their records and palette blocks are words of
    // their brick's block, already in `words`.
    let live_bytes = branches * 16 + words * 4;
    let per = |n: usize| n as f64 / unit as f64;
    let ratio = |a: usize, b: usize| if b == 0 { 0.0 } else { a as f64 / b as f64 };
    let mib = |b: usize| b as f64 / (1 << 20) as f64;
    println!("memory: {name}");
    println!("  chunks          {chunks}");
    println!(
        "  bytes held {bytes} ({:.2} MiB), live {live_bytes} ({:.2} MiB, nodes x16 + words x4)",
        mib(bytes),
        mib(live_bytes)
    );
    println!(
        "  nodes {branches}, leaves {leaves}, live words {words}, \
         free nodes {free_nodes}, free words {free_words}"
    );
    println!(
        "  per unit: {:.3} bytes held, {:.3} live bytes, {:.4} nodes, {:.4} leaves, {:.3} live words",
        per(bytes),
        per(live_bytes),
        per(branches),
        per(leaves),
        per(words)
    );
    println!(
        "  fragmentation: free_words / words = {:.4}, free share of word storage {:.2}%",
        ratio(free_words, words),
        ratio(free_words, words + free_words) * 100.0
    );
    let b = tree.memory_breakdown();
    println!(
        "  breakdown: {} upper nodes, {} child words, {} bricks ({} long headers), \
         {} default leaves, {} default partial, {} other full, {} single records, \
         {} palette records, palette words {:?}; live {} bytes",
        b.upper_nodes,
        b.child_words,
        b.bricks,
        b.long_headers,
        b.default_leaves,
        b.default_partial_leaves,
        b.full_records,
        b.single_records,
        b.palette_records,
        b.palette_words,
        b.live_bytes()
    );
}
