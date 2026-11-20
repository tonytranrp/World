//! Determinism, seams, coarse-versus-fine agreement, bounds, the chunk source
//! and the config, checked through the public API only.

use std::sync::OnceLock;

use proptest::prelude::*;
use world_core::consts::{WORLD_END_VOXEL, WORLD_MIN_VOXEL};
use world_core::{ConfigAsset, MaterialId, MaterialTable, VoxelPos};
use world_gen::{
    CHUNK_SIZE, ChunkColumns, ChunkCoord, ChunkSource, Generator, GeneratorVersion, TerrainConfig,
};
use world_testkit::profile::proptest_config;

const SEED: u64 = 0xC0FFEE;

fn generator() -> &'static Generator {
    static G: OnceLock<Generator> = OnceLock::new();
    G.get_or_init(|| {
        Generator::new(SEED, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap()
    })
}

fn chunk(x: u32, z: u32) -> ChunkCoord {
    ChunkCoord::new(x, 512, z).unwrap()
}

/// A chunk on a steep, varied site and its x and z neighbours.
const SITE: (u32, u32) = (511, 518);

fn site() -> &'static [ChunkColumns; 3] {
    static C: OnceLock<[ChunkColumns; 3]> = OnceLock::new();
    C.get_or_init(|| {
        let g = generator();
        [
            g.columns(chunk(SITE.0, SITE.1)),
            g.columns(chunk(SITE.0 + 1, SITE.1)),
            g.columns(chunk(SITE.0, SITE.1 + 1)),
        ]
    })
}

fn source() -> &'static ChunkSource {
    static S: OnceLock<ChunkSource> = OnceLock::new();
    S.get_or_init(|| ChunkSource::new(site()[0].clone()))
}

#[test]
fn the_same_inputs_give_the_same_columns_on_any_thread() {
    let g = generator();
    let c = chunk(700, 300);
    let expected = g.columns(c);
    let again = Generator::new(SEED, GeneratorVersion::CURRENT, TerrainConfig::default())
        .unwrap()
        .columns(c);
    assert_eq!(expected, again);
    let results: Vec<ChunkColumns> = std::thread::scope(|s| {
        // Different threads, and a different order of other work first.
        let handles: Vec<_> = (0..4)
            .map(|i| {
                s.spawn(move || {
                    for j in 0..i {
                        let _ = g.column(j * 1000, -j * 77);
                    }
                    g.columns(c)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for r in &results {
        assert_eq!(r, &expected);
    }
}

#[test]
fn different_seeds_versions_and_configs_are_different_identities() {
    let a = Generator::new(1, GeneratorVersion::V1, TerrainConfig::default()).unwrap();
    let b = Generator::new(2, GeneratorVersion::V1, TerrainConfig::default()).unwrap();
    let mut tuned = TerrainConfig::default();
    tuned.regional.relief_height_m = 0.5;
    let c = Generator::new(1, GeneratorVersion::V1, tuned).unwrap();
    assert_ne!(a.id(), b.id());
    assert_ne!(a.id(), c.id());
    assert_eq!(a.id().version, GeneratorVersion::V1);
    let k = chunk(512, 512);
    assert_ne!(a.columns(k).columns(), b.columns(k).columns());
}

#[test]
fn the_terrain_is_varied_and_layered() {
    let (lo, hi) = site()[0].height_range();
    assert!(hi - lo > 64, "the site should not be flat: {lo}..{hi}");
    // A survey of the whole world, one column every 256 m.
    let g = generator();
    let mut surfaces = std::collections::BTreeMap::new();
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for j in 0..64 {
        for i in 0..64 {
            let c = g
                .column(
                    WORLD_MIN_VOXEL + i * 16_384 + 77,
                    WORLD_MIN_VOXEL + j * 16_384 + 31,
                )
                .unwrap();
            *surfaces.entry(c.surface.0).or_insert(0) += 1;
            lo = lo.min(c.height);
            hi = hi.max(c.height);
        }
    }
    println!(
        "surfaces {surfaces:?}, heights {} m to {} m",
        lo / 64,
        hi / 64
    );
    assert_eq!(
        surfaces.len(),
        3,
        "grass, dirt and stone all occur: {surfaces:?}"
    );
    assert!(hi - lo > 100 * 64, "at least 100 m of relief: {lo}..{hi}");
}

#[test]
fn neighbouring_chunks_meet_without_a_seam() {
    let [here, east, north] = site();
    let n = CHUNK_SIZE;
    let g = generator();
    let min = here.chunk().min_voxel();
    let mut worst_inside = 0;
    let mut worst_border = 0;
    for i in 0..n {
        // Border columns equal the point query of the same column.
        let edge = *here.column(n - 1, i).unwrap();
        assert_eq!(
            edge,
            g.column(min.x + n as i32 - 1, min.z + i as i32).unwrap()
        );
        assert_eq!(
            *east.column(0, i).unwrap(),
            g.column(min.x + n as i32, min.z + i as i32).unwrap()
        );
        let step = |a: i32, b: i32| (a - b).abs();
        worst_border = worst_border
            .max(step(edge.height, east.column(0, i).unwrap().height))
            .max(step(
                here.column(i, n - 1).unwrap().height,
                north.column(i, 0).unwrap().height,
            ));
        for k in n - 64..n - 1 {
            worst_inside = worst_inside
                .max(step(
                    here.column(k, i).unwrap().height,
                    here.column(k + 1, i).unwrap().height,
                ))
                .max(step(
                    here.column(i, k).unwrap().height,
                    here.column(i, k + 1).unwrap().height,
                ));
        }
    }
    assert!(
        worst_border <= worst_inside.max(1),
        "a step of {worst_border} across the border, at most {worst_inside} inside"
    );
}

#[test]
fn coarse_samples_are_the_full_resolution_columns() {
    let g = generator();
    let cols = &site()[0];
    for level in 1..=5 {
        let coarse = g.coarse_columns(cols.chunk(), level).unwrap();
        let (n, s) = (coarse.samples_per_axis(), coarse.spacing());
        assert_eq!(n * s, CHUNK_SIZE);
        for j in 0..n {
            for i in 0..n {
                assert_eq!(
                    coarse.column(i, j),
                    cols.column(i * s, j * s),
                    "level {level}, sample ({i}, {j})"
                );
            }
        }
    }
}

#[test]
fn the_default_config_round_trips_through_ron_and_validates() {
    let config = TerrainConfig::default();
    assert_eq!(config.validate(), Ok(()));
    let text = ron::ser::to_string_pretty(&config, ron::ser::PrettyConfig::default()).unwrap();
    let back: TerrainConfig = ron::from_str(&text).unwrap();
    assert_eq!(back, config);
    assert_eq!(back.fingerprint(), config.fingerprint());
    assert_eq!(TerrainConfig::PATH, "config/terrain.ron");
    // Unknown fields are refused, so a typo cannot silently fall back.
    assert!(ron::from_str::<TerrainConfig>(&text.replacen("regional", "regionnal", 1)).is_err());
}

#[test]
fn default_materials_exist_in_the_shipped_table() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../assets/config/materials.ron"
    );
    let table: MaterialTable = ron::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let config = TerrainConfig::default();
    assert_eq!(config.check_materials(&table), Ok(()));
    let s = &config.surface;
    for (name, id) in [("stone", s.stone), ("dirt", s.dirt), ("grass", s.grass)] {
        assert_eq!(table.by_name(name).unwrap().id, id, "{name}");
    }
}

#[test]
fn chunk_source_leaves_match_column_materials() {
    let src = source();
    let cols = src.columns();
    let min = cols.chunk().min_voxel();
    let (lo, _) = cols.height_range();
    let mut voxels = [MaterialId::AIR; 64];
    for (i, k) in [(0, 0), (100, 900), (1020, 1020), (512, 3)] {
        let h = cols.column(i, k).unwrap().height;
        for y in [h - 2, h - 70, lo - 300] {
            let at = VoxelPos::new(min.x + i as i32, y, min.z + k as i32);
            src.leaf(at, &mut voxels);
            for (slot, m) in voxels.iter().enumerate() {
                let (x, yy, z) = (slot as i32 & 3, (slot as i32 >> 2) & 3, slot as i32 >> 4);
                let p = VoxelPos::new(at.x + x, at.y + yy, at.z + z);
                assert_eq!(
                    Some(*m),
                    cols.material_at(p).or(Some(MaterialId::AIR)),
                    "{p:?}"
                );
            }
        }
    }
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn point_queries_equal_chunk_columns(x in 0u32..CHUNK_SIZE, z in 0u32..CHUNK_SIZE) {
        let cols = &site()[0];
        let min = cols.chunk().min_voxel();
        let column = generator().column(min.x + x as i32, min.z + z as i32).unwrap();
        prop_assert_eq!(Some(&column), cols.column(x, z));
        let y = column.height - 3;
        prop_assert_eq!(
            generator().material(VoxelPos::new(min.x + x as i32, y, min.z + z as i32)).unwrap(),
            cols.material(x, y, z).unwrap()
        );
    }

    #[test]
    fn heights_stay_within_the_configured_bounds(
        seed in any::<u64>(),
        x in WORLD_MIN_VOXEL..WORLD_END_VOXEL,
        z in WORLD_MIN_VOXEL..WORLD_END_VOXEL,
    ) {
        let g = Generator::new(seed, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap();
        let b = g.height_bounds();
        let c = g.column(x, z).unwrap();
        prop_assert!(b.min <= c.height && c.height <= b.max, "{} not in {:?}", c.height, b);
        // Every solid voxel has a material.
        for dy in 1..8 {
            prop_assert!(!c.material_at(c.height - dy, &g.layers()).is_air());
        }
        prop_assert!(c.material_at(c.height, &g.layers()).is_air());
    }

    #[test]
    fn chunk_source_uniform_answers_are_exact(
        level in 0u32..=5,
        cx in 0u32..1024,
        cz in 0u32..1024,
        dy in -2_000i32..2_000,
        unaligned in 0u32..3,
    ) {
        let src = source();
        let cols = src.columns();
        let size = 1u32 << (2 * level);
        // Aligned cubes use the pyramid; shifted ones are scanned.
        let shift = if unaligned == 0 && size < CHUNK_SIZE { 1 } else { 0 };
        let x = (cx / size * size + shift).min(CHUNK_SIZE - size);
        let z = (cz / size * size).min(CHUNK_SIZE - size);
        let (lo, hi) = cols.height_range();
        let y = (lo + hi) / 2 + dy;
        let min = cols.chunk().min_voxel();
        let at = VoxelPos::new(min.x + x as i32, y, min.z + z as i32);
        if let Some(m) = src.uniform(at, size) {
            // Check the claim column by column at the cube's top and bottom,
            // which bound every voxel in between for layered columns.
            for k in z..z + size {
                for i in x..x + size {
                    let c = cols.column(i, k).unwrap();
                    let top = y + size as i32 - 1;
                    prop_assert_eq!(c.material_at(top, &cols.layers()), m);
                    prop_assert_eq!(c.material_at(y, &cols.layers()), m);
                    if !m.is_air() {
                        prop_assert!(top < c.bedrock_below());
                    }
                }
            }
        }
        prop_assert_eq!(src.uniform(at, 0), None);
    }
}
