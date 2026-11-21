//! Golden hashes of generated terrain for fixed seeds, versions and chunks.
//!
//! These values were recorded on Windows x86_64 and must reproduce exactly on
//! every supported platform (CI runs this file on Windows x86_64, macOS arm64
//! and macOS x86_64 under Rosetta; decision 0008). A mismatch means generated
//! output changed: either a bug (platform-dependent float behaviour, an
//! accidental change) or a deliberate change, which must come with a new
//! `GeneratorVersion` and newly recorded values here.
//!
//! Each hash is xxh3-64 (World_Testkit's `StateHasher`, little-endian) over
//! the generator id (seed, version, config fingerprint), the chunk coordinate
//! and every column's height, surface material, surface depth and soil depth
//! in index order. Version 2 adds caves, which columns do not show, so its
//! cases hash materials of voxels inside the cave band.

use world_core::VoxelPos;
use world_gen::noise::{Fractal, Fractal3, FractalParams, Perlin};
use world_gen::{ChunkCoord, Column, Generator, GeneratorId, GeneratorVersion, TerrainConfig};
use world_testkit::hash::StateHasher;

fn generator(seed: u64) -> Generator {
    Generator::new(seed, GeneratorVersion::V1, TerrainConfig::default()).expect("valid default")
}

fn chunk(x: u32, y: u32, z: u32) -> ChunkCoord {
    ChunkCoord::new(x, y, z).expect("inside the world")
}

fn hash_id(h: &mut StateHasher, id: GeneratorId, coord: ChunkCoord) {
    h.write_u64(id.seed)
        .write_u32(id.version.0)
        .write_u64(id.config);
    for c in coord.to_array() {
        h.write_u32(c);
    }
}

fn hash_columns(h: &mut StateHasher, columns: &[Column]) {
    for c in columns {
        h.write_i32(c.height)
            .write_u16(c.surface.0)
            .write_u8(c.surface_depth)
            .write_u8(c.soil_depth);
    }
}

fn chunk_hash(seed: u64, coord: ChunkCoord) -> u64 {
    let g = generator(seed);
    let cols = g.columns(coord);
    let mut h = StateHasher::new();
    hash_id(&mut h, cols.generator(), coord);
    hash_columns(&mut h, &cols.to_vec());
    h.finish()
}

fn coarse_hash(seed: u64, coord: ChunkCoord, level: u32) -> u64 {
    let g = generator(seed);
    let cols = g.coarse_columns(coord, level).expect("valid level");
    let mut h = StateHasher::new();
    hash_id(&mut h, cols.generator(), coord);
    h.write_u32(level);
    hash_columns(&mut h, cols.columns());
    h.finish()
}

/// Compares and prints every value, so a failure shows all of them at once
/// (and recording new values is a copy from the output).
fn check(cases: &[(&str, u64, u64)]) {
    let mut bad = Vec::new();
    for &(name, actual, expected) in cases {
        println!("{name}: 0x{actual:016X}");
        if actual != expected {
            bad.push(format!(
                "{name}: got 0x{actual:016X}, recorded 0x{expected:016X}"
            ));
        }
    }
    assert!(bad.is_empty(), "golden hashes differ:\n{}", bad.join("\n"));
}

#[test]
fn generated_chunks_match_recorded_hashes() {
    check(&[
        (
            "seed 0, chunk at the world origin",
            chunk_hash(0, chunk(512, 512, 512)),
            0xD8CC_580B_9584_4687,
        ),
        (
            "seed 0xDEADBEEF, world corner chunk (negative coordinates)",
            chunk_hash(0xDEAD_BEEF, chunk(0, 0, 0)),
            0xC8E7_0651_ACDE_4212,
        ),
        (
            "seed 42, last chunk on x",
            chunk_hash(42, chunk(1023, 700, 3)),
            0xD20E_6608_6638_2E6F,
        ),
    ]);
}

#[test]
fn coarse_samples_match_recorded_hashes() {
    let c = chunk(300, 512, 777);
    check(&[
        (
            "seed 7, level 1",
            coarse_hash(7, c, 1),
            0x8521_7696_C80C_2FEC,
        ),
        (
            "seed 7, level 2",
            coarse_hash(7, c, 2),
            0xB7FE_EDD6_2668_0E0C,
        ),
        (
            "seed 7, level 4",
            coarse_hash(7, c, 4),
            0x9119_4777_C616_4466,
        ),
    ]);
}

#[test]
fn point_queries_match_recorded_hashes() {
    let g = generator(0x5EED);
    let mut h = StateHasher::new();
    for i in 0..256i32 {
        let x = i.wrapping_mul(40_503) % 524_288;
        let z = -(i.wrapping_mul(9_973) % 524_288);
        let column = g.column(x, z).expect("inside the world");
        hash_columns(&mut h, &[column]);
        for dy in [-200, -3, -1, 0, 1] {
            let m = g
                .material(VoxelPos::new(x, column.height + dy, z))
                .expect("inside the world");
            h.write_u16(m.0);
        }
    }
    check(&[(
        "seed 0x5EED, 256 columns",
        h.finish(),
        0x176D_C1B5_0FF0_72AF,
    )]);
}

#[test]
fn noise_primitives_match_recorded_values() {
    let mut h = StateHasher::new();
    let reference = Perlin::reference();
    let seeded = Perlin::from_seed(1);
    let fractal = Fractal::new(2, &FractalParams::new(16.0, 5));
    for i in 0..64 {
        let x = i as f32 * 1.37 - 40.0;
        let y = i as f32 * -0.61 + 7.25;
        h.write_f32(reference.noise3(x, y, x * 0.5))
            .write_f32(seeded.noise2(x, y))
            .write_f32(fractal.fbm(x * 97.0, y * 31.0))
            .write_f32(fractal.ridged(x * 97.0, y * 31.0));
    }
    check(&[("noise primitives", h.finish(), 0x01ED_282B_FE8E_B0F3)]);
}

#[test]
fn version_2_caves_match_recorded_hashes() {
    let g = Generator::new(0x5EED, GeneratorVersion::V2, TerrainConfig::default())
        .expect("valid default");
    assert!(g.has_caves());
    let mut h = StateHasher::new();
    h.write_u64(g.id().config);
    let mut carved = 0;
    for i in 0..512i32 {
        let x = i.wrapping_mul(40_503) % 524_288;
        let z = -(i.wrapping_mul(9_973) % 524_288);
        let column = g.column(x, z).expect("inside the world");
        // A vertical run of voxels 4 to 20 m down, one every 16 cm.
        for k in 0..100 {
            let y = column.height - 256 - k * 10;
            let m = g
                .material(VoxelPos::new(x, y, z))
                .expect("inside the world");
            carved += u32::from(m.is_air());
            h.write_u16(m.0);
        }
    }
    assert!(carved > 0, "the probes cross caves");
    let fractal = Fractal3::new(2, &FractalParams::new(16.0, 3), 1.75);
    for i in 0..64 {
        let x = i as f32 * 977.0 - 40_000.0;
        h.write_f32(fractal.fbm(x, x * -0.37, x * 0.61));
    }
    check(&[(
        "seed 0x5EED, version 2 cave band",
        h.finish(),
        0xB452_794E_995B_7C87,
    )]);
}
