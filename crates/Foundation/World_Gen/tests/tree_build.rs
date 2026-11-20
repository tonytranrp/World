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
