//! Bytes of real chunks in the CPU tree: the full-detail chunks of decisions
//! 0020, 0021 and 0025 (every chunk the default terrain's surface crosses,
//! and one below, whose footprint centre lies within 64 m of the origin) and
//! the coarse ring of decision 0020 (cell level 2, footprints from 64 to
//! 384 m). Prints figures; asserts only that the chunks were built.
//!
//! Run with
//! `cargo test --release -p world_gen --test chunk_bytes -- --ignored --nocapture`.

use std::collections::{BTreeMap, BTreeSet};

use world_core::consts::TREE_DEPTH;
use world_core::tree::{CHUNK_LEVEL, MemoryBreakdown, NodeRef};
use world_core::{MaterialId, VoxelPos, VoxelTree};
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
    total.full_words += b.full_words;
    total.default_partial_leaves += b.default_partial_leaves;
    total.single_records += b.single_records;
    total.palette_records += b.palette_records;
    total.pair_leaves += b.pair_leaves;
    total.pair_headers += b.pair_headers;
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

/// Leaf masks of every brick under `node` (a child at `level`): those of the
/// partial leaves of the brick's default material (the most common full
/// leaf material, ties to the lower id) into `defaults`, every partial
/// leaf's into `all`.
fn leaf_masks(node: NodeRef<'_>, level: u32, defaults: &mut Vec<u64>, all: &mut Vec<u64>) {
    let NodeRef::Branch(b) = node else {
        return;
    };
    if level > 1 {
        for (_, child) in b.children() {
            leaf_masks(child, level - 1, defaults, all);
        }
        return;
    }
    let mut counts = BTreeMap::<MaterialId, u32>::new();
    for (_, leaf) in b.children() {
        if let NodeRef::Uniform(m) = leaf {
            *counts.entry(m).or_default() += 1;
        }
    }
    let default = counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(m, _)| *m);
    for (_, leaf) in b.children() {
        if let NodeRef::Leaf(l) = leaf {
            all.push(l.mask());
            if let Some(d) = default
                && l.palette().collect::<Vec<_>>() == [d]
            {
                defaults.push(l.mask());
            }
        }
    }
}

/// Distinct values per chunk and over all chunks, and how many of the
/// values the most common 256, 4,096 and 65,536 masks of all chunks cover.
fn census(name: &str, per_chunk: &[Vec<u64>]) {
    let mut distinct: Vec<usize> = per_chunk
        .iter()
        .map(|masks| masks.iter().collect::<BTreeSet<_>>().len())
        .collect();
    distinct.sort_unstable();
    let total: usize = per_chunk.iter().map(Vec::len).sum();
    let mut counts = BTreeMap::<u64, usize>::new();
    for &m in per_chunk.iter().flatten() {
        *counts.entry(m).or_default() += 1;
    }
    let mut by_count: Vec<usize> = counts.values().copied().collect();
    by_count.sort_unstable_by(|a, b| b.cmp(a));
    let cover = |n: usize| by_count.iter().take(n).sum::<usize>() as f64 / total as f64 * 100.0;
    let pct = |p: usize| distinct[(distinct.len() - 1) * p / 100];
    let sum_distinct: usize = distinct.iter().sum();
    println!(
        "{name}: {total} masks, {} distinct over all chunks; per chunk distinct median {} p90 {} max {} (sum {sum_distinct}); \
         the most common 256 / 4,096 / 65,536 cover {:.1}% / {:.1}% / {:.1}%",
        counts.len(),
        pct(50),
        pct(90),
        distinct[distinct.len() - 1],
        cover(256),
        cover(4096),
        cover(65536)
    );
}

/// How repetitive leaf masks are, for a dictionary of masks (decision
/// 0031's next step).
#[test]
#[ignore = "prints figures; run by hand"]
fn leaf_mask_census() {
    let generator = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
        .expect("the default config is valid");
    let (mut defaults, mut all) = (Vec::new(), Vec::new());
    for coord in chunks(&generator, 0, 64) {
        let mut tree = VoxelTree::new();
        tree.build_region(
            CHUNK_LEVEL,
            coord.min_voxel(),
            &generator.chunk_source(coord),
        )
        .expect("an aligned chunk");
        let (mut d, mut a) = (Vec::new(), Vec::new());
        leaf_masks(tree.root(), TREE_DEPTH - 1, &mut d, &mut a);
        defaults.push(d);
        all.push(a);
    }
    census("default partial leaves", &defaults);
    census("all partial leaves", &all);
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
