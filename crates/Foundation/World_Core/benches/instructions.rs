//! Instruction counts for the tree's hot operations, measured with gungraun
//! under Valgrind's Callgrind. Counts are deterministic to a fraction of a
//! percent even on shared CI runners, where wall time varies several-fold, so
//! CI gates on these rather than on Criterion's timings.
//!
//! Linux only (Valgrind). Run with `cargo xtask bench --instructions`, which
//! needs `valgrind` and a `gungraun-runner` of the same version as the
//! `gungraun` crate on PATH. On other systems this bench is an empty `main`.
//!
//! Inputs come from fixed seeds and are built in each benchmark's argument
//! expression, which gungraun evaluates outside the measured region. Each
//! benchmark returns what it built, so dropping it is not measured either.

#[cfg(target_os = "linux")]
mod common;

#[cfg(target_os = "linux")]
use std::hint::black_box;

#[cfg(target_os = "linux")]
use common::{Terrain, dense_block, random_edits, striped_block};
#[cfg(target_os = "linux")]
use gungraun::prelude::*;
#[cfg(target_os = "linux")]
use world_core::serialize::{decode, encode};
#[cfg(target_os = "linux")]
use world_core::{Edit, MaterialId, TreeRead, TreeSnapshot, VoxelBox, VoxelPos, VoxelTree};

/// The dense block's edge and material count. Valgrind runs code about 50x
/// slower, so scenes are smaller than in the Criterion benches.
#[cfg(target_os = "linux")]
const DENSE_EDGE: u32 = 32;
#[cfg(target_os = "linux")]
const DENSE_MATERIALS: u16 = 8;

/// A full block of mixed materials and 1,000 edits inside it, all to
/// materials the block does not contain, so every edit changes its voxel.
#[cfg(target_os = "linux")]
fn dense_with_edits() -> (VoxelTree, Vec<Edit>) {
    let tree = dense_block(VoxelPos::ORIGIN, DENSE_EDGE, DENSE_MATERIALS, 11);
    let edits = random_edits(1_000, VoxelPos::ORIGIN, DENSE_EDGE, 4, 12)
        .into_iter()
        .map(|e| Edit::new(e.pos, MaterialId(e.material.0 + DENSE_MATERIALS)))
        .collect();
    (tree, edits)
}

#[cfg(target_os = "linux")]
fn terrain() -> VoxelTree {
    Terrain::new(64, 7).build()
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::empty_tree((VoxelTree::new(), random_edits(1_000, VoxelPos::new(-128, -128, -128), 256, 8, 1)))]
#[bench::dense_block(dense_with_edits())]
fn set_1k(input: (VoxelTree, Vec<Edit>)) -> VoxelTree {
    let (mut tree, edits) = input;
    for e in &edits {
        tree.set(black_box(e.pos), e.material).unwrap();
    }
    tree
}

#[cfg(target_os = "linux")]
fn probes() -> Vec<VoxelPos> {
    random_edits(10_000, VoxelPos::new(0, -32, 0), 64, 1, 5)
        .into_iter()
        .map(|e| e.pos)
        .collect()
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::terrain((terrain(), probes()))]
fn get_10k(input: (VoxelTree, Vec<VoxelPos>)) -> (usize, VoxelTree) {
    let (tree, probes) = input;
    let solid = probes
        .iter()
        .filter(|&&p| !tree.get(black_box(p)).is_air())
        .count();
    (solid, tree)
}

/// Random boxes up to 32 voxels per axis, a quarter of them carving air.
#[cfg(target_os = "linux")]
fn boxes() -> Vec<(VoxelBox, MaterialId)> {
    let mut rng = common::Lcg(3);
    (0..64)
        .map(|_| {
            let size = [0; 3].map(|_| 1 + rng.below(32));
            let min = rng.pos_in(VoxelPos::new(-128, -128, -128), 256);
            (
                VoxelBox::from_min_size(min, size),
                MaterialId(rng.below(4) as u16),
            )
        })
        .collect()
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::random_64(boxes())]
fn fill_box_64(boxes: Vec<(VoxelBox, MaterialId)>) -> VoxelTree {
    let mut tree = VoxelTree::new();
    for &(region, material) in &boxes {
        tree.fill_box(black_box(region), material).unwrap();
    }
    tree
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::dense_block(dense_with_edits())]
fn apply_1k(input: (VoxelTree, Vec<Edit>)) -> VoxelTree {
    let (mut tree, edits) = input;
    black_box(tree.apply(black_box(&edits)).unwrap());
    tree
}

// A snapshot followed by one edit: the edit pays the copy-on-write cost of
// everything the snapshot still shares. (A plain comment: the
// `library_benchmark` attribute rejects doc comments.)
#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::striped_128(striped_block(128))]
fn snapshot_then_edit(mut tree: VoxelTree) -> (VoxelTree, TreeSnapshot) {
    let snapshot = tree.snapshot();
    tree.set(black_box(VoxelPos::new(61, 62, 63)), MaterialId(3))
        .unwrap();
    (tree, snapshot)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::terrain(terrain())]
#[bench::dense_block(dense_block(VoxelPos::ORIGIN, DENSE_EDGE, DENSE_MATERIALS, 11))]
fn encode_tree(tree: VoxelTree) -> (Vec<u8>, VoxelTree) {
    (encode(black_box(&tree)), tree)
}

#[cfg(target_os = "linux")]
#[library_benchmark]
#[bench::terrain(encode(&terrain()))]
#[bench::dense_block(encode(&dense_block(VoxelPos::ORIGIN, DENSE_EDGE, DENSE_MATERIALS, 11)))]
fn decode_tree(bytes: Vec<u8>) -> (VoxelTree, Vec<u8>) {
    (decode(black_box(&bytes)).unwrap(), bytes)
}

#[cfg(target_os = "linux")]
library_benchmark_group!(
    name = tree_ops,
    benchmarks = [
        set_1k,
        get_10k,
        fill_box_64,
        apply_1k,
        snapshot_then_edit,
        encode_tree,
        decode_tree
    ]
);

#[cfg(target_os = "linux")]
main!(library_benchmark_groups = tree_ops);

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!(
        "the instruction-count benchmarks need Linux and Valgrind; \
         see `cargo xtask bench --instructions`"
    );
}
