//! Generated chunks built straight into the world tree (decisions 0006 and
//! 0008): the tree holds exactly the generator's voxels, in canonical form,
//! as one chunk.

use world_core::tree::CHUNK_LEVEL;
use world_core::{MaterialId, VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, Generator, GeneratorVersion, TerrainConfig};
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
    let source = g.chunk_source(coord);
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
    let source = g.chunk_source(coord);
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
            // Buried cells take the column's top material (decision 0029).
            let want = source
                .buried(i, y, j)
                .unwrap_or_else(|| column.material_at(y + cell / 2, &source.columns().layers()));
            // Every voxel of the cell has the cell's material.
            for d in [0, 5, 15] {
                let q = VoxelPos::new(p.x + d, p.y + 15 - d, p.z + d);
                assert_eq!(tree.get(q), want, "cell {i},{j},{k} at {q:?}");
            }
        }
    }
    // Far less storage than the same chunk at full detail.
    let mut full = VoxelTree::new();
    full.build_region(CHUNK_LEVEL, min, &g.chunk_source(coord))
        .unwrap();
    assert!(tree.stats().bytes * 8 < full.stats().bytes);
}

#[test]
fn coarse_cells_that_differ_from_their_sample_are_never_seen() {
    use world_gen::CoarseSource;
    let g = generator();
    let coord = surface_chunk(&g, 520, 505);
    let source = CoarseSource::new(g.coarse_columns(coord, 2).unwrap());
    let min = coord.min_voxel();
    let mut tree = VoxelTree::new();
    tree.build_region(CHUNK_LEVEL, min, &source).unwrap();
    let cell = 16;
    let n = 64;
    let at = |i: i32, k: i32, j: i32| {
        let (i, j) = (i.clamp(0, n - 1), j.clamp(0, n - 1));
        tree.get(VoxelPos::new(
            min.x + i * cell,
            min.y + k * cell,
            min.z + j * cell,
        ))
    };
    // Whether a cell's sample is solid, from the columns (the cell above
    // the chunk's top row is in the next chunk, which is not built).
    let solid = |i: i32, k: i32, j: i32| {
        let (i, j) = (i.clamp(0, n - 1), j.clamp(0, n - 1));
        let column = source.columns().column(i as u32, j as u32).unwrap();
        let y = min.y + k * cell + cell / 2;
        !column.material_at(y, &source.columns().layers()).is_air()
    };
    let (mut buried, mut collapsed) = (0, 0);
    for j in 0..n {
        for i in 0..n {
            let column = *source.columns().column(i as u32, j as u32).unwrap();
            for k in 0..n {
                let y = min.y + k * cell;
                let sample = column.material_at(y + cell / 2, &source.columns().layers());
                let got = at(i, k, j);
                if source.buried(i as u32, y, j as u32).is_some() {
                    buried += 1;
                    collapsed += usize::from(got != sample);
                    // Solid above and on every side (outside the footprint,
                    // the nearest sample stands in), so no ray reaches it.
                    for (di, dk, dj) in [(0, 1, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)] {
                        assert!(solid(i + di, k + dk, j + dj), "cell {i},{k},{j}");
                    }
                    assert!(!got.is_air());
                } else {
                    assert_eq!(got, sample, "visible cell {i},{k},{j}");
                }
            }
        }
    }
    assert!(
        buried > 0 && collapsed > 0,
        "{buried} buried, {collapsed} changed"
    );
}

/// Builds `coord` with caves and checks columns of voxels against the
/// generator's point queries; returns the tree and how many solid voxels the
/// checked columns lost to caves.
fn check_cave_chunk(g: &Generator, coord: ChunkCoord) -> (VoxelTree, usize) {
    let source = g.chunk_source(coord);
    assert!(source.has_caves());
    let min = coord.min_voxel();
    let mut tree = VoxelTree::new();
    tree.build_region(CHUNK_LEVEL, min, &source).unwrap();
    check_canonical(&tree).unwrap();
    let layers = g.layers();
    let size = CHUNK_SIZE as i32;
    let mut carved = 0;
    for z in (0..size).step_by(53) {
        for x in (0..size).step_by(47) {
            let column = source.columns().column(x as u32, z as u32).unwrap();
            for y in 0..size {
                let p = VoxelPos::new(min.x + x, min.y + y, min.z + z);
                let want = g.material(p).unwrap();
                assert_eq!(tree.get(p), want, "at {p:?}");
                if want.is_air() && !column.material_at(p.y, &layers).is_air() {
                    carved += 1;
                }
            }
        }
    }
    (tree, carved)
}

#[test]
fn caves_carve_underground_chunks_exactly() {
    let g = generator();
    assert!(g.has_caves());
    // Well below the surface, inside the cave band.
    let surface = surface_chunk(&g, 511, 518);
    let deep = ChunkCoord::new(surface.x(), surface.y() - 4, surface.z()).unwrap();
    let (tree, carved) = check_cave_chunk(&g, deep);
    assert!(carved > 0, "the checked columns cross a cave");
    let stats = tree.stats();
    assert!(stats.leaves > 0);
    // Rock and open caves stay whole subtrees: far fewer leaves than a
    // leaf per brick.
    assert!(stats.leaves < 64 * 64 * 64 * 64 / 4, "{stats:?}");
}

#[test]
fn caves_keep_the_surface_cover_on_surface_chunks() {
    let g = generator();
    let coord = surface_chunk(&g, 300, 700);
    let (tree, _) = check_cave_chunk(&g, coord);
    // The top `cover_m` of every column is untouched.
    let min = coord.min_voxel();
    let cover = (g.config().caves.cover_m * 64.0) as i32;
    let columns = g.columns(coord);
    for (x, z) in [(0u32, 0u32), (500, 600), (1023, 17)] {
        let c = columns.column(x, z).unwrap();
        for d in 1..=cover.min(64) {
            let p = VoxelPos::new(min.x + x as i32, c.height - d, min.z + z as i32);
            if ChunkCoord::containing(p) == Some(coord) {
                assert!(!tree.get(p).is_air(), "cover carved at {p:?}");
            }
        }
    }
}

#[test]
fn version_1_has_no_caves() {
    let g = Generator::new(0xC0FFEE, GeneratorVersion::V1, TerrainConfig::default()).unwrap();
    assert!(!g.has_caves());
    let coord = surface_chunk(&g, 511, 518);
    assert!(!g.chunk_source(coord).has_caves());
    let v2 = generator();
    assert_ne!(
        g.id().config,
        v2.id().config,
        "caves are part of v2's identity"
    );
}
