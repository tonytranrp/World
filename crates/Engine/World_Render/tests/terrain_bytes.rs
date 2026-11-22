//! Bytes of real terrain chunks in the CPU tree and in GPU brick blocks
//! (decisions 0020, 0021 and 0025). Prints figures; asserts nothing beyond the
//! encoder agreeing with the tree.
//!
//! Run with
//! `cargo test --release -p world_render --test terrain_bytes -- --ignored --nocapture`.

use world_core::tree::{CHUNK_LEVEL, NodeRef};
use world_core::{MaterialId, VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, Generator, GeneratorVersion, TerrainConfig};
use world_render::internals::format::brick::encode_brick;
use world_render::internals::format::summary::Tally;

/// Every full-detail chunk the default terrain's surface crosses, and one
/// below, whose footprint centre lies within 64 m of the origin: the chunks
/// decision 0020 measured.
fn chunks(generator: &Generator) -> Vec<ChunkCoord> {
    let origin = ChunkCoord::containing(VoxelPos::new(0, 0, 0)).unwrap();
    let radius = 64 * 64; // voxels
    let mut out = Vec::new();
    for dz in -6i32..=6 {
        for dx in -6i32..=6 {
            let (cx, cz) = (origin.x() as i32 + dx, origin.z() as i32 + dz);
            let footprint = ChunkCoord::new(cx as u32, origin.y(), cz as u32).unwrap();
            let min = footprint.min_voxel();
            let half = CHUNK_SIZE as i64 / 2;
            let (ox, oz) = (i64::from(min.x) + half, i64::from(min.z) + half);
            if ox * ox + oz * oz > radius * radius {
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

#[derive(Default, Debug)]
struct Census {
    bricks: usize,
    /// Bricks with a full one-material leaf that is not their default.
    bricks_with_other_full: usize,
    /// Surface records (one material with air) of the brick's default.
    single_of_default: usize,
    single: usize,
    /// Bricks with partial leaves of their default but no other full leaf,
    /// by how many: one, two, three or more (decision 0025's rule).
    default_partial_only: [usize; 3],
    /// Surface records in bricks without any full leaf (so no default).
    single_without_full: usize,
    two_material: usize,
}

fn census(node: NodeRef<'_>, level: u32, out: &mut Census) {
    let NodeRef::Branch(b) = node else {
        return;
    };
    if level > 1 {
        for (_, child) in b.children() {
            census(child, level - 1, out);
        }
        return;
    }
    out.bricks += 1;
    // The default: the most common full leaf material, ties to the lower id.
    let mut counts = std::collections::BTreeMap::<MaterialId, u32>::new();
    for (_, leaf) in b.children() {
        if let NodeRef::Uniform(m) = leaf {
            *counts.entry(m).or_default() += 1;
        }
    }
    let default = counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0)))
        .map(|(m, _)| *m);
    if counts.len() > 1 {
        out.bricks_with_other_full += 1;
    }
    let mut of_default = 0;
    for (_, leaf) in b.children() {
        if let NodeRef::Leaf(l) = leaf {
            let palette: Vec<_> = l.palette().collect();
            match palette.len() {
                1 => {
                    out.single += 1;
                    if Some(palette[0]) == default {
                        out.single_of_default += 1;
                        of_default += 1;
                    }
                    if counts.is_empty() {
                        out.single_without_full += 1;
                    }
                }
                2 => out.two_material += 1,
                _ => {}
            }
        }
    }
    if counts.len() == 1 && of_default > 0 {
        out.default_partial_only[of_default.min(3) - 1] += 1;
    }
}

/// GPU block words of every brick under `node`.
fn gpu_words(node: NodeRef<'_>, level: u32, words: &mut Vec<u32>, tally: &mut Tally) -> u64 {
    let NodeRef::Branch(b) = node else {
        return 0;
    };
    if level == 1 {
        return u64::from(encode_brick(node, 0, false, words, tally).words);
    }
    b.children()
        .map(|(_, c)| gpu_words(c, level - 1, words, tally))
        .sum()
}

#[test]
#[ignore = "prints figures; run by hand"]
fn terrain_chunk_bytes() {
    let generator = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
        .expect("the default config is valid");
    let coords = chunks(&generator);
    let mut sizes = Vec::new();
    let mut total = world_core::tree::MemoryBreakdown::default();
    let mut census_all = Census::default();
    let (mut gpu_total, mut held) = (0u64, 0usize);
    let mut words = Vec::new();
    let mut tally = Tally::default();
    for &coord in &coords {
        let mut tree = VoxelTree::new();
        tree.build_region(
            CHUNK_LEVEL,
            coord.min_voxel(),
            &generator.chunk_source(coord),
        )
        .expect("an aligned chunk");
        let chunk = tree.take_chunk(coord.min_voxel()).expect("a chunk");
        let mut tree = VoxelTree::new();
        tree.insert_chunk(coord.min_voxel(), chunk).ok();
        let b = tree.memory_breakdown();
        held += tree.chunks().iter().map(|c| c.stats.bytes).sum::<usize>();
        let chunk_live = b.live_bytes();
        sizes.push(chunk_live);
        census(
            tree.root(),
            world_core::consts::TREE_DEPTH - 1,
            &mut census_all,
        );
        words.clear();
        gpu_total += 4 * gpu_words(
            tree.root(),
            world_core::consts::TREE_DEPTH - 1,
            &mut words,
            &mut tally,
        );
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
    }
    sizes.sort_unstable();
    let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
    let pct = |p: usize| sizes[(sizes.len() - 1) * p / 100] / 1024;
    println!("chunks: {}", coords.len());
    println!(
        "CPU live: {:.1} MiB, held {:.1} MiB; per chunk KiB median {} p10 {} p90 {}",
        mib(sizes.iter().sum()),
        mib(held),
        pct(50),
        pct(10),
        pct(90)
    );
    println!("{total:#?}");
    println!("{census_all:#?}");
    println!(
        "GPU brick blocks: {:.1} MiB ({} KiB a chunk)",
        gpu_total as f64 / (1024.0 * 1024.0),
        gpu_total / 1024 / coords.len() as u64
    );
}
