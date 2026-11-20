//! Generated chunks built straight into the world tree (decisions 0006 and
//! 0008): the tree holds exactly the generator's voxels, in canonical form,
//! as one chunk.

use world_core::tree::CHUNK_LEVEL;
use world_core::{MaterialId, VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, ChunkSource, Generator, GeneratorVersion, TerrainConfig};
use world_testkit::dense::check_canonical;

fn generator() -> Generator {
    Generator::new(
        0xC0FFEE,
        GeneratorVersion::CURRENT,
        TerrainConfig::default(),
    )
    .unwrap()
}

/// The chunk at column chunk (`x`, `z`) whose height range holds the middle
/// of its surface.
fn surface_chunk(g: &Generator, x: u32, z: u32) -> ChunkCoord {
    let probe = g.columns(ChunkCoord::new(x, 512, z).unwrap());
    let (lo, hi) = probe.height_range();
    let min = probe.chunk().min_voxel();
    ChunkCoord::containing(VoxelPos::new(min.x, lo + (hi - lo) / 2, min.z)).unwrap()
}

#[test]
fn a_generated_chunk_builds_into_the_tree_exactly() {
    let g = generator();
    let coord = surface_chunk(&g, 511, 518);
    let source = ChunkSource::new(g.columns(coord));
    let min = coord.min_voxel();

    let mut tree = VoxelTree::new();
    assert_eq!(tree.build_region(CHUNK_LEVEL, min, &source), Ok(true));
    let stats = tree.stats();
    assert_eq!(stats.chunks, 1, "a surface chunk is one mixed chunk");
    assert!(stats.leaves > 0);
    check_canonical(&tree).unwrap();

    // Every voxel of a lattice of columns, top to bottom, matches the
    // generator's own per-voxel answer.
    let size = CHUNK_SIZE as i32;
    for z in (0..size).step_by(37) {
        for x in (0..size).step_by(41) {
            for y in 0..size {
                let p = VoxelPos::new(min.x + x, min.y + y, min.z + z);
                assert_eq!(tree.get(p), g.material(p).unwrap(), "at {p:?}");
            }
        }
    }
    // Neighbouring chunks were not touched.
    let outside = VoxelPos::new(min.x - 1, min.y, min.z);
    assert_eq!(tree.get(outside), MaterialId::AIR);
}

#[test]
fn a_loaded_chunk_replaces_a_not_resident_region() {
    let g = generator();
    let coord = surface_chunk(&g, 300, 700);
    let source = ChunkSource::new(g.columns(coord));
    let min = coord.min_voxel();

    let mut tree = VoxelTree::unloaded();
    assert!(!tree.is_resident(min));
    assert_eq!(tree.lookup(min), None);
    tree.build_region(CHUNK_LEVEL, min, &source).unwrap();
    assert!(tree.is_resident(min));
    assert_eq!(tree.lookup(min), Some(g.material(min).unwrap()));
    let neighbour = VoxelPos::new(min.x + CHUNK_SIZE as i32, min.y, min.z);
    assert!(!tree.is_resident(neighbour), "only the built chunk loaded");

    assert_eq!(tree.unload(CHUNK_LEVEL, min), Ok(true));
    assert!(!tree.is_resident(min));
    let s = tree.stats();
    assert_eq!((s.chunks, s.branches), (0, 0), "unloading frees everything");
}

#[test]
fn a_coarse_chunk_is_uniform_cells_matching_its_samples() {
    use world_gen::CoarseSource;
    let g = generator();
    let coord = surface_chunk(&g, 520, 505);
    let level = 2; // 16-voxel cells, a quarter metre
    let source = CoarseSource::new(g.coarse_columns(coord, level).unwrap());
    let min = coord.min_voxel();
    let mut tree = VoxelTree::new();
    tree.build_region(CHUNK_LEVEL, min, &source).unwrap();
    check_canonical(&tree).unwrap();
    assert_eq!(tree.stats().leaves, 0, "cells are whole bricks or larger");
    let cell = 16;
    for (i, j) in [(0, 0), (13, 40), (63, 63), (31, 7)] {
        let column = *source.columns().column(i, j).unwrap();
        for k in 0..64 {
            let y = min.y + k * cell;
            let p = VoxelPos::new(min.x + i as i32 * cell, y, min.z + j as i32 * cell);
            let want = column.material_at(y + cell / 2, &source.columns().layers());
            // Every voxel of the cell has the cell's material.
            for d in [0, 5, 15] {
                let q = VoxelPos::new(p.x + d, p.y + 15 - d, p.z + d);
                assert_eq!(tree.get(q), want, "cell {i},{j},{k} at {q:?}");
            }
        }
    }
    // Far less storage than the same chunk at full detail.
    let mut full = VoxelTree::new();
    full.build_region(CHUNK_LEVEL, min, &ChunkSource::new(g.columns(coord)))
        .unwrap();
    assert!(tree.stats().bytes * 8 < full.stats().bytes);
}
