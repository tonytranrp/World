//! Bytes of real chunks in the CPU tree: the full-detail chunks of decisions
//! 0020, 0021 and 0025 (every chunk the default terrain's surface crosses,
//! and one below, whose footprint centre lies within 64 m of the origin) and
//! the coarse ring of decision 0020 (cell level 2, footprints from 64 to
//! 384 m). Prints figures; asserts only that the chunks were built.
//!
//! Run with
//! `cargo test --release -p world_gen --test chunk_bytes -- --ignored --nocapture`.

use world_core::tree::{CHUNK_LEVEL, MemoryBreakdown};
use world_core::{VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, CoarseSource, Generator, GeneratorVersion, TerrainConfig};

/// The chunks of every footprint whose centre lies within `(inner, outer]`
/// metres of the origin: those the surface crosses and one below.
fn chunks(generator: &Generator, inner_m: i64, outer_m: i64) -> Vec<ChunkCoord> {
    let origin = ChunkCoord::containing(VoxelPos::new(0, 0, 0)).unwrap();
    let reach = outer_m / 16 + 1;
    let mut out = Vec::new();
    for dz in -reach..=reach {
        for dx in -reach..=reach {
            let (cx, cz) = (origin.x() as i64 + dx, origin.z() as i64 + dz);
            let footprint = ChunkCoord::new(cx as u32, origin.y(), cz as u32).unwrap();
            let min = footprint.min_voxel();
            let half = CHUNK_SIZE as i64 / 2;
            let (ox, oz) = (i64::from(min.x) + half, i64::from(min.z) + half);
            let d2 = ox * ox + oz * oz;
            let (inner, outer) = (inner_m * 64, outer_m * 64);
            if d2 > outer * outer || (inner > 0 && d2 <= inner * inner) {
                continue;
            }
            let (lo, hi) = generator.columns(footprint).height_range();
            let bottom = ChunkCoord::containing(VoxelPos::new(min.x, lo, min.z)).unwrap();
            let top = ChunkCoord::containing(VoxelPos::new(min.x, hi, min.z)).unwrap();
            for y in bottom.y() - 1..=top.y() {
                out.push(ChunkCoord::new(cx as u32, y, cz as u32).unwrap());
            }
        }
    }
    out
}

fn add(total: &mut MemoryBreakdown, b: &MemoryBreakdown) {
    total.upper_nodes += b.upper_nodes;
    total.bricks += b.bricks;
    total.long_headers += b.long_headers;
    total.default_leaves += b.default_leaves;
    total.child_words += b.child_words;
    total.full_records += b.full_records;
    total.default_partial_leaves += b.default_partial_leaves;
    total.single_records += b.single_records;
    total.palette_records += b.palette_records;
    for i in 0..4 {
        total.palette_words[i] += b.palette_words[i];
    }
    total.free_words += b.free_words;
    total.held_bytes += b.held_bytes;
}

/// Builds every chunk with `source` as the workers do (`build_region`, then
/// `take_chunk` into a tree of its own) and prints its bytes.
fn report<S: world_core::RegionSource>(
    name: &str,
    coords: &[ChunkCoord],
    source: impl Fn(ChunkCoord) -> S,
) {
    let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
    let mut sizes = Vec::new();
    let mut total = MemoryBreakdown::default();
    let started = std::time::Instant::now();
    for &coord in coords {
        let mut tree = VoxelTree::new();
        tree.build_region(CHUNK_LEVEL, coord.min_voxel(), &source(coord))
            .expect("an aligned chunk");
        let chunk = tree.take_chunk(coord.min_voxel()).expect("a chunk");
        let mut tree = VoxelTree::new();
        tree.insert_chunk(coord.min_voxel(), chunk).ok();
        let mut b = tree.memory_breakdown();
        // The chunk's own pages, not the small top store above it.
        b.held_bytes = tree.chunks().iter().map(|c| c.stats.bytes).sum();
        sizes.push(b.live_bytes());
        add(&mut total, &b);
    }
    sizes.sort_unstable();
    let pct = |p: usize| sizes[(sizes.len() - 1) * p / 100] / 1024;
    println!(
        "{name}: {} chunks built in {:.1?}",
        coords.len(),
        started.elapsed()
    );
    println!(
        "  live {:.1} MiB, pages held {:.1} MiB; per chunk KiB median {} p10 {} p90 {}",
        mib(total.live_bytes()),
        mib(total.held_bytes),
        pct(50),
        pct(10),
        pct(90)
    );
    println!("  {total:?}");
}

#[test]
#[ignore = "prints figures; run by hand"]
fn real_chunk_bytes() {
    let generator = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
        .expect("the default config is valid");
    let full = chunks(&generator, 0, 64);
    report("full detail within 64 m", &full, |c| {
        generator.chunk_source(c)
    });
    let coarse = chunks(&generator, 64, 384);
    report("coarse cell level 2, 64 to 384 m", &coarse, |c| {
        CoarseSource::new(generator.coarse_columns(c, 2).expect("a valid level"))
    });
    assert!(!full.is_empty() && !coarse.is_empty());
}
