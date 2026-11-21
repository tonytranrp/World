//! What a residency worker spends per real terrain chunk: building it and
//! computing its summaries (decisions 0022 and 0024). Prints figures;
//! asserts only that the summaries exist.
//!
//! Run with
//! `cargo test --release -p world_render --test terrain_summaries -- --ignored --nocapture`.
//! `WORLD_SUMMARY_ROUNDS` sets how many times every chunk is built (3).

use std::time::{Duration, Instant};

use world_core::tree::{CHUNK_LEVEL, ChunkData};
use world_core::{VoxelPos, VoxelTree};
use world_gen::{CHUNK_SIZE, ChunkCoord, CoarseSource, Generator, GeneratorVersion, TerrainConfig};
use world_render::internals::summaries::ChunkSummaries;

/// The chunks `terrain_bytes` measures: every full-detail chunk the default
/// terrain's surface crosses, and one below, whose footprint centre lies
/// within 64 m of the origin.
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

/// Builds a chunk as the workers do (`build_region`, `take_chunk`).
/// `None` when the chunk is air or one material.
fn build(min: VoxelPos, source: &impl world_core::tree::RegionSource) -> Option<ChunkData> {
    let mut tree = VoxelTree::new();
    tree.build_region(CHUNK_LEVEL, min, source)
        .expect("an aligned chunk");
    tree.take_chunk(min).ok()
}

#[derive(Default)]
struct Times {
    /// Columns (the heightfield) once per footprint, which the workers'
    /// column cache shares between the chunks stacked on it.
    columns: Duration,
    build: Vec<Duration>,
    summaries: Vec<Duration>,
    nodes: usize,
}

impl Times {
    fn report(&self, name: &str, rounds: usize) {
        let chunks = self.build.len() / rounds;
        let ms = |d: &[Duration]| {
            let mut v: Vec<f64> = d.iter().map(|d| d.as_secs_f64() * 1e3).collect();
            v.sort_by(f64::total_cmp);
            (v.iter().sum::<f64>() / v.len() as f64, v[v.len() / 2])
        };
        let (b, bm) = ms(&self.build);
        let (s, sm) = ms(&self.summaries);
        let c = self.columns.as_secs_f64() * 1e3 / self.build.len() as f64;
        println!(
            "{name}: {chunks} chunks, {} nodes a chunk; columns {c:.2} ms a chunk, build {b:.2} ms \
             (median {bm:.2}), summaries {s:.3} ms (median {sm:.3}), summaries/(columns + build) {:.1}%",
            self.nodes / chunks,
            100.0 * s / (c + b),
        );
    }
}

#[test]
#[ignore = "prints figures; run by hand"]
fn terrain_chunk_worker_time() {
    let generator = Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default())
        .expect("the default config is valid");
    let coords = chunks(&generator);
    let rounds: usize = std::env::var("WORLD_SUMMARY_ROUNDS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(3);
    let (mut full, mut coarse) = (Times::default(), Times::default());
    for round in 0..rounds {
        let mut columns: Option<(ChunkCoord, world_gen::ChunkColumns)> = None;
        let mut cells: Option<(ChunkCoord, world_gen::CoarseColumns)> = None;
        for &coord in &coords {
            let min = coord.min_voxel();
            let stacked = |c: &ChunkCoord| (c.x(), c.z()) == (coord.x(), coord.z());
            if !columns.as_ref().is_some_and(|(c, _)| stacked(c)) {
                let t = Instant::now();
                columns = Some((coord, generator.columns(coord)));
                full.columns += t.elapsed();
            }
            if !cells.as_ref().is_some_and(|(c, _)| stacked(c)) {
                let t = Instant::now();
                let level = generator.coarse_columns(coord, 2).expect("a valid level");
                cells = Some((coord, level));
                coarse.columns += t.elapsed();
            }
            let (_, footprint) = columns.as_ref().expect("computed above");
            let t = Instant::now();
            let source = generator
                .chunk_source_from(footprint, coord)
                .expect("the same generator and footprint");
            let data = build(min, &source).expect("a mixed chunk");
            let built = t.elapsed();
            let t = Instant::now();
            let s = ChunkSummaries::compute(min, &data).expect("surface chunks are mixed");
            let summarized = t.elapsed();
            if round == 0 {
                full.nodes += s.len();
            }
            full.build.push(built);
            full.summaries.push(summarized);

            let (_, level) = cells.as_ref().expect("computed above");
            let t = Instant::now();
            let Some(data) = build(min, &CoarseSource::new(level.clone())) else {
                continue;
            };
            let built = t.elapsed();
            let t = Instant::now();
            let s = ChunkSummaries::compute(min, &data);
            let summarized = t.elapsed();
            if round == 0 {
                coarse.nodes += s.map_or(0, |s| s.len());
            }
            coarse.build.push(built);
            coarse.summaries.push(summarized);
        }
    }
    full.report("full detail", rounds);
    coarse.report("coarse", rounds);
}
