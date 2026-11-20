//! Residency end to end, without Bevy: loading around the player, edits
//! surviving eviction, reload and a crash, edits to unloaded chunks, the
//! coarse tier and the threaded pool (decision 0006).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use world_core::consts::VOXELS_PER_METER;
use world_core::serialize::wal::{WalRecord, WalWriter, read_log};
use world_core::{Edit, MaterialId, VoxelBox, VoxelPos};
use world_gen::TerrainConfig;

use super::config::ResidencyConfig;
use super::world::StreamedWorld;

/// The time between updates in these tests, in seconds.
const FRAME: f32 = 1.0 / 60.0;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("world_residency_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn config(workers: u32) -> ResidencyConfig {
    ResidencyConfig {
        full_radius_m: 16.0,
        evict_radius_m: 24.0,
        coarse_radius_m: 0.0,
        workers,
        grafts_per_frame: 64,
        ..ResidencyConfig::default()
    }
}

fn open(dir: &Path, config: ResidencyConfig) -> StreamedWorld {
    StreamedWorld::open(config, TerrainConfig::default(), dir).unwrap()
}

/// Two metres above the ground at (`x`, `z`), in voxels.
fn eye_at(world: &StreamedWorld, x: i32, z: i32) -> VoxelPos {
    let ground = world.generator().column(x, z).unwrap().height;
    VoxelPos::new(x, ground + 2 * VOXELS_PER_METER as i32, z)
}

/// Updates until everything around `eye` is loaded and nothing is in flight.
fn settle(world: &mut StreamedWorld, eye: VoxelPos) {
    for _ in 0..2000 {
        world.update(eye, FRAME);
        let s = world.stats();
        if world.is_settled(eye) && s.loading == 0 && s.saving == 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!(
        "the world never settled around {eye:?}: {:?}",
        world.stats()
    );
}

/// A small dug ball just below `eye`'s ground.
fn dig(world: &mut StreamedWorld, centre: VoxelPos) -> Vec<VoxelPos> {
    let mut edits = Vec::new();
    for z in -4..=4 {
        for y in -4..=4 {
            for x in -4..=4 {
                if x * x + y * y + z * z <= 16 {
                    edits.push(Edit::new(
                        VoxelPos::new(centre.x + x, centre.y + y, centre.z + z),
                        MaterialId::AIR,
                    ));
                }
            }
        }
    }
    world.apply(&edits).unwrap();
    edits.iter().map(|e| e.pos).collect()
}

fn ground_below(world: &StreamedWorld, eye: VoxelPos) -> VoxelPos {
    let ground = world.generator().column(eye.x, eye.z).unwrap().height;
    VoxelPos::new(eye.x, ground - 3, eye.z)
}

#[test]
fn the_sphere_around_the_player_loads_the_generated_terrain() {
    let dir = temp_dir("sphere");
    let mut world = open(&dir, config(0));
    let eye = eye_at(&world, 1000, -3000);
    settle(&mut world, eye);
    let s = world.stats();
    assert!(s.loaded >= 7, "{s:?}");
    assert_eq!(s.edited, 0);
    let snapshot = world.snapshot();
    let radius = super::plan::voxels(16.0);
    let mut checked = 0;
    for (dx, dz) in [(0, 0), (300, -200), (-500, 400), (900, 0)] {
        for dy in [-200, -3, 0, 5, 200] {
            let p = VoxelPos::new(eye.x + dx, eye.y + dy, eye.z + dz);
            let chunk = world_gen::ChunkCoord::containing(p).unwrap();
            if super::plan::chunk_distance(eye, chunk) > radius {
                // Outside the sphere: never loaded by this plan.
                continue;
            }
            assert_eq!(
                snapshot.lookup(p),
                Some(world.generator().material(p).unwrap()),
                "{p:?}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 5);
    let changes = world.take_changes();
    assert_eq!(changes.loaded.len(), s.loaded);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn edits_survive_eviction_and_come_back() {
    let dir = temp_dir("evict");
    let mut world = open(&dir, config(0));
    let eye = eye_at(&world, 2000, 2000);
    settle(&mut world, eye);
    let target = ground_below(&world, eye);
    let hole = dig(&mut world, target);
    assert!(hole.iter().all(|&p| world.snapshot().get(p).is_air()));
    assert!(world.stats().edited >= 1);

    // Walk 200 m away: the edited chunk is saved and evicted.
    let away = eye_at(&world, eye.x + 200 * 64, eye.z);
    settle(&mut world, away);
    assert_eq!(world.snapshot().lookup(hole[0]), None, "evicted");
    let s = world.stats();
    assert!(s.saved_chunks >= 1 && s.warm_chunks >= 1, "{s:?}");

    // Back again: the warm copy rejoins with the hole.
    settle(&mut world, eye);
    assert!(hole.iter().all(|&p| world.snapshot().get(p).is_air()));

    // A fresh session reads the region file instead.
    drop(world);
    let mut world = open(&dir, config(0));
    assert!(world.stats().saved_chunks >= 1);
    settle(&mut world, eye);
    assert!(hole.iter().all(|&p| world.snapshot().get(p).is_air()));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn edits_survive_a_crash_through_the_log() {
    let dir = temp_dir("crash");
    let eye;
    let hole;
    let cube = VoxelBox::from_min_size(VoxelPos::new(0, 0, 0), [8; 3]);
    {
        let mut world = open(&dir, config(0));
        eye = eye_at(&world, -7000, 500);
        settle(&mut world, eye);
        let target = ground_below(&world, eye);
        hole = dig(&mut world, target);
        let above = VoxelPos::new(eye.x, eye.y + 64, eye.z);
        world
            .fill_box(VoxelBox::from_min_size(above, [8; 3]), MaterialId(1))
            .unwrap();
        // One frame syncs the log; then the process "crashes" (no save).
        world.update(eye, FRAME);
    }
    let log = std::fs::read(dir.join("edits.wvl")).unwrap();
    assert_eq!(read_log(&log).unwrap().0.len(), 2, "both edits logged");

    let mut world = open(&dir, config(0));
    assert!(
        world.stats().pending_chunks >= 1,
        "the log replays as waiting edits"
    );
    settle(&mut world, eye);
    assert!(hole.iter().all(|&p| world.snapshot().get(p).is_air()));
    let above = VoxelPos::new(eye.x + 3, eye.y + 67, eye.z + 3);
    assert_eq!(world.snapshot().get(above), MaterialId(1));
    assert_eq!(world.stats().pending_chunks, 0);
    assert!(
        world.stats().edited >= 1,
        "edits applied from the log are not on disk, so their chunks must be saved"
    );
    let _ = cube;
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn edits_to_unloaded_chunks_wait_and_save_all_keeps_them() {
    let dir = temp_dir("pending");
    let far = VoxelPos::new(100_000, 0, -100_000);
    {
        let mut world = open(&dir, config(0));
        let eye = eye_at(&world, 0, 0);
        settle(&mut world, eye);
        let ground = world.generator().column(far.x, far.z).unwrap().height;
        let target = VoxelPos::new(far.x, ground - 1, far.z);
        world.apply(&[Edit::new(target, MaterialId(3))]).unwrap();
        assert_eq!(world.snapshot().lookup(target), None, "not loaded here");
        assert!(world.stats().pending_chunks >= 1);
        world.save_all().unwrap();
        assert_eq!(world.stats().pending_chunks, 0);
    }
    let log = std::fs::read(dir.join("edits.wvl")).unwrap();
    assert!(
        read_log(&log).unwrap().0.is_empty(),
        "save_all empties the log"
    );

    let mut world = open(&dir, config(0));
    assert_eq!(world.stats().pending_chunks, 0);
    let eye = eye_at(&world, far.x, far.z);
    settle(&mut world, eye);
    let ground = world.generator().column(far.x, far.z).unwrap().height;
    assert_eq!(
        world
            .snapshot()
            .get(VoxelPos::new(far.x, ground - 1, far.z)),
        MaterialId(3)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn coarse_columns_surround_the_player() {
    let dir = temp_dir("coarse");
    let mut world = open(
        &dir,
        ResidencyConfig {
            coarse_radius_m: 48.0,
            coarse_cell_level: 3,
            ..config(0)
        },
    );
    let eye = eye_at(&world, 5000, 5000);
    for _ in 0..50 {
        world.update(eye, FRAME);
    }
    let s = world.stats();
    assert!(s.coarse_columns >= 9, "{s:?}");
    let coarse = world.coarse_snapshot();
    let ground = world.generator().column(eye.x, eye.z).unwrap().height;
    // Well below the surface is solid in the coarse world, well above is air.
    assert!(
        !coarse
            .get(VoxelPos::new(eye.x, ground - 200, eye.z))
            .is_air()
    );
    assert!(
        coarse
            .get(VoxelPos::new(eye.x, ground + 400, eye.z))
            .is_air()
    );
    assert!(world.take_changes().coarse);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_threaded_pool_gives_the_same_world() {
    let dir = temp_dir("threads");
    let mut inline = open(&dir, config(0));
    let eye = eye_at(&inline, -2500, 7000);
    settle(&mut inline, eye);
    let mut threaded = open(&temp_dir("threads2"), config(3));
    settle(&mut threaded, eye);
    assert_eq!(inline.stats().loaded, threaded.stats().loaded);
    let (a, b) = (inline.snapshot(), threaded.snapshot());
    for dx in (-900..900).step_by(97) {
        for dy in (-300..300).step_by(41) {
            let p = VoxelPos::new(eye.x + dx, eye.y + dy, eye.z + dx / 2);
            assert_eq!(a.lookup(p), b.lookup(p), "{p:?}");
        }
    }
    drop(threaded);
    std::fs::remove_dir_all(&dir).unwrap();
    let _ = std::fs::remove_dir_all(temp_dir("threads2"));
}

#[test]
fn an_overlay_from_another_generator_version_is_refused() {
    use super::jobs;
    use super::store::SavedChunk;
    use world_core::serialize::region::RecordKind;
    let dir = temp_dir("version");
    let world = open(&dir, config(0));
    let coord = world_gen::ChunkCoord::new(500, 512, 500).unwrap();
    let saved = [SavedChunk {
        kind: RecordKind::Overlay,
        generator: 99,
        bytes: vec![1, 2, 3],
    }];
    let generators = jobs::Generators::new(Arc::new(world.generator().clone()));
    let loaded = jobs::load(&generators, coord, None, &saved);
    assert!(loaded.refused, "the generated chunk stands in");
    assert!(loaded.error.unwrap().contains("version 99"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_render_tree_shows_loaded_chunks_and_coarse_beyond() {
    let dir = temp_dir("render");
    let mut world = open(
        &dir,
        ResidencyConfig {
            coarse_radius_m: 48.0,
            coarse_cell_level: 2,
            ..config(0)
        },
    );
    let eye = eye_at(&world, 3000, -4000);
    settle(&mut world, eye);
    for _ in 0..30 {
        world.update(eye, FRAME);
    }
    let render = world.render_snapshot();
    let full = world.snapshot();
    // Loaded: the render tree reads exactly like the world.
    let near = VoxelPos::new(eye.x + 7, eye.y - 140, eye.z - 3);
    assert_eq!(render.get(near), full.get(near));
    // Beyond the loaded sphere but inside the coarse ring: coarse terrain.
    let ground = world
        .generator()
        .column(eye.x + 2500, eye.z)
        .unwrap()
        .height;
    let far_below = VoxelPos::new(eye.x + 2500, ground - 300, eye.z);
    assert_eq!(full.lookup(far_below), None, "not loaded at full detail");
    assert!(!render.get(far_below).is_air(), "coarse cells fill it");
    let changes = world.take_changes();
    assert!(!changes.render_bricks.is_empty());

    // Edits reach the render tree, and their bricks are reported.
    let target = ground_below(&world, eye);
    let hole = dig(&mut world, target);
    assert!(
        hole.iter()
            .all(|&p| world.render_snapshot().get(p).is_air())
    );
    let changes = world.take_changes();
    assert!(changes.render_bricks.contains(&target.brick()));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The GPU cache's stage 1, run on the CPU, fed exactly as the plugin feeds
/// it while the world streams in: it refines down to bricks around the eye.
#[test]
fn the_gpu_cache_refines_the_streamed_world_to_bricks() {
    use world_render::internals::BrickCacheConfig;
    use world_render::internals::batch::UploadBatch;
    use world_render::internals::frame::{FrameInput, run_frame};
    use world_render::internals::key::Shape;
    use world_render::internals::layout::PoolLayout;
    use world_render::internals::residency::Residency;
    use world_render::internals::select::ViewPoint;
    let dir = temp_dir("gpu_feed");
    let mut world = open(
        &dir,
        ResidencyConfig {
            coarse_radius_m: 96.0,
            ..config(0)
        },
    );
    let eye = eye_at(&world, 0, 0);
    let shape = Shape::of(&world.render_snapshot());
    let layout = PoolLayout {
        node_count: 1 << 22,
        segments: vec![64 << 20, 64 << 20],
        brick_ids: 1 << 18,
        allocs_per_segment: 1 << 18,
        node_groups: 1 << 20,
        upload_budget: 24 << 20,
        request_capacity: 1024,
        victim_capacity: 1024,
    };
    let mut cache = Residency::new(shape, &layout).unwrap();
    let config = BrickCacheConfig::default();
    // 1440 rows at a 70 degree field of view.
    let view = ViewPoint {
        eye: [eye.x as f32, eye.y as f32, eye.z as f32],
        pixel_angle: 2.0 / (1.43 * 1440.0),
    };
    let mut batch = UploadBatch::default();
    for frame in 1..=120 {
        world.update(eye, FRAME);
        let snapshot = world.render_snapshot();
        let dirty = world_core::DirtyBricks::from_unsorted(world.take_changes().render_bricks);
        let input = FrameInput {
            source: &snapshot,
            frame,
            refresh: frame == 1,
            dirty: dirty.as_slice(),
            view: Some(view),
            requests: &[],
            victims: &[],
            config: &config,
            budget: 24 << 20,
            victim_capacity: 64,
        };
        run_frame(&mut cache, &input, &mut batch);
    }
    let stats = cache.stats();
    assert!(stats.bricks > 1000, "{stats:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The lowest voxel of the chunk holding `eye`, which every plan loads.
fn eye_chunk_min(eye: VoxelPos) -> VoxelPos {
    world_gen::ChunkCoord::containing(eye).unwrap().min_voxel()
}

#[test]
fn destruction_shows_from_afar_once_its_chunk_is_written() {
    let dir = temp_dir("coarse_edit");
    let mut world = open(
        &dir,
        ResidencyConfig {
            full_radius_m: 24.0,
            evict_radius_m: 32.0,
            coarse_radius_m: 64.0,
            coarse_cell_level: 2,
            ..config(0)
        },
    );
    let eye = eye_at(&world, 6000, -6000);
    settle(&mut world, eye);
    for _ in 0..30 {
        world.update(eye, FRAME);
    }
    // A 1 m cube 2.5 m under the ground, aligned to coarse cells.
    let ground = world.generator().column(eye.x, eye.z).unwrap().height;
    let min = VoxelPos::new(eye.x & !63, (ground - 160) & !63, eye.z & !63);
    let centre = VoxelPos::new(min.x + 32, min.y + 32, min.z + 32);
    assert!(
        !world.coarse_snapshot().get(centre).is_air(),
        "the generated coarse terrain is solid there"
    );
    assert!(world.snapshot().lookup(centre).is_some(), "loaded");
    world
        .fill_box(VoxelBox::from_min_size(min, [64; 3]), MaterialId::AIR)
        .unwrap();
    assert!(world.render_snapshot().get(centre).is_air());

    // 56 m away the chunk is evicted and written, and the coarse ring
    // shows its new summary instead of the generated one.
    let away = eye_at(&world, eye.x + 56 * 64, eye.z);
    settle(&mut world, away);
    for _ in 0..10 {
        world.update(away, FRAME);
    }
    assert_eq!(world.snapshot().lookup(centre), None, "evicted");
    assert!(world.stats().saved_chunks >= 1);
    assert!(
        world.coarse_snapshot().get(centre).is_air(),
        "the coarse chunk is the saved summary"
    );
    assert!(
        world.render_snapshot().get(centre).is_air(),
        "the render tree draws the hole from afar"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_overlay_from_an_older_generator_keeps_its_edits() {
    use super::jobs::{self, Generators};
    use super::store::RegionStore;
    use world_core::VoxelTree;
    use world_core::serialize::content_hash;
    use world_core::serialize::region::RecordKind;
    use world_core::tree::ChunkData;
    use world_gen::{ChunkCoord, Generator, GeneratorVersion};

    let terrain = TerrainConfig::default();
    let seed = ResidencyConfig::default().seed;
    let v1 = Arc::new(Generator::new(seed, GeneratorVersion::V1, terrain.clone()).unwrap());
    let v2 = Arc::new(Generator::new(seed, GeneratorVersion::V2, terrain).unwrap());
    let old = Generators::new(Arc::clone(&v1));
    let now = Generators::new(Arc::clone(&v2));
    let tree_of = |coord: ChunkCoord, data: ChunkData| {
        let mut tree = VoxelTree::unloaded();
        tree.insert_chunk(coord.min_voxel(), data).unwrap();
        tree
    };
    // A chunk as version 1 made it, then edited.
    let edited_v1 = |coord: ChunkCoord, edits: &[Edit]| {
        let mut tree = tree_of(coord, jobs::generate(&v1, coord));
        tree.apply(edits).unwrap();
        tree
    };

    // Deep down, where caves make the versions differ, the overlay is merged
    // onto the version 1 base.
    let (x, z) = (-4000, 9000);
    let ground = v2.column(x, z).unwrap().height;
    let mut checked = false;
    for depth_m in [24, 40, 56, 72, 88, 104, 120] {
        let p = VoxelPos::new(x, ground - depth_m * VOXELS_PER_METER as i32, z);
        let coord = ChunkCoord::containing(p).unwrap();
        let a = content_hash(&tree_of(coord, jobs::generate(&v1, coord)));
        let b = content_hash(&tree_of(coord, jobs::generate(&v2, coord)));
        if a == b {
            continue;
        }
        let expected = edited_v1(coord, &[Edit::new(p, MaterialId(3))]);
        let data = expected.chunk_data(coord.min_voxel()).unwrap();
        let records = jobs::encode_chunk(&old, coord, data, 2).records;
        assert_eq!(
            (records[0].kind, records[0].generator),
            (RecordKind::Overlay, 1)
        );
        let loaded = jobs::load(&now, coord, None, &records[..1]);
        assert!(!loaded.refused, "{:?}", loaded.error);
        assert_eq!(
            content_hash(&tree_of(coord, loaded.data)),
            content_hash(&expected)
        );
        checked = true;
        break;
    }
    assert!(checked, "no chunk where the versions differ");

    // In the world: the old overlay's edit loads, and editing the chunk again
    // keeps it.
    let dir = temp_dir("old_version");
    let world = open(&dir, config(0));
    let eye = eye_at(&world, x, z);
    drop(world);
    let coord = ChunkCoord::containing(eye).unwrap();
    let cmin = coord.min_voxel();
    let first = VoxelPos::new(cmin.x + 5, cmin.y + 5, cmin.z + 5);
    assert_ne!(v1.material(first).unwrap(), MaterialId(3));
    let expected = edited_v1(coord, &[Edit::new(first, MaterialId(3))]);
    let records = jobs::encode_chunk(&old, coord, expected.chunk_data(cmin).unwrap(), 2).records;
    RegionStore::open(&dir)
        .unwrap()
        .write(coord, records)
        .unwrap();

    let mut world = open(&dir, config(0));
    settle(&mut world, eye);
    assert_eq!(world.snapshot().get(first), MaterialId(3), "the old edit");
    assert!(world.take_errors().is_empty());
    assert_eq!(world.stats().edited, 0, "loaded as saved");
    let second = VoxelPos::new(cmin.x + 9, cmin.y + 9, cmin.z + 9);
    world.apply(&[Edit::new(second, MaterialId(4))]).unwrap();
    world.save_all().unwrap();
    drop(world);

    let mut world = open(&dir, config(0));
    settle(&mut world, eye);
    assert_eq!(world.snapshot().get(first), MaterialId(3), "kept");
    assert_eq!(world.snapshot().get(second), MaterialId(4));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_refused_record_is_set_aside_before_its_chunk_is_written() {
    use super::store::{RegionStore, SavedChunk};
    use world_core::serialize::region::RecordKind;
    let dir = temp_dir("set_aside");
    let world = open(&dir, config(0));
    let eye = eye_at(&world, 7000, 7000);
    drop(world);
    let coord = world_gen::ChunkCoord::containing(eye).unwrap();
    let record = SavedChunk {
        kind: RecordKind::Overlay,
        generator: 99,
        bytes: vec![7; 64],
    };
    let store = RegionStore::open(&dir).unwrap();
    store.write(coord, vec![record.clone()]).unwrap();

    let mut world = open(&dir, config(0));
    settle(&mut world, eye);
    let errors = world.take_errors();
    assert!(
        errors.iter().any(|e| e.contains("version 99")),
        "{errors:?}"
    );
    let target = VoxelPos::new(eye.x, eye.y + 30, eye.z);
    world.apply(&[Edit::new(target, MaterialId(3))]).unwrap();
    world.save_all().unwrap();

    let aside: Vec<PathBuf> = std::fs::read_dir(store.set_aside_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    let name = aside[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.contains("overlay.v99"), "{name}");
    assert_eq!(std::fs::read(&aside[0]).unwrap(), record.bytes);
    drop(world);

    let mut world = open(&dir, config(0));
    settle(&mut world, eye);
    assert_eq!(world.snapshot().get(target), MaterialId(3));
    assert!(world.take_errors().is_empty());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn chunks_are_written_only_when_they_change() {
    let dir = temp_dir("amplification");
    let mut world = open(&dir, config(0));
    let eye = eye_at(&world, -1500, 4500);
    settle(&mut world, eye);
    let target = VoxelPos::new(eye.x, eye.y + 30, eye.z);
    world.apply(&[Edit::new(target, MaterialId(3))]).unwrap();
    let away = eye_at(&world, eye.x + 200 * 64, eye.z);
    settle(&mut world, away);
    let writes = world.stats().writes;
    assert!(writes >= 1);

    // Back from the warm cache and away again: nothing changed, nothing is
    // written.
    settle(&mut world, eye);
    assert_eq!(world.snapshot().get(target), MaterialId(3));
    assert_eq!(world.stats().edited, 0);
    settle(&mut world, away);
    assert_eq!(world.stats().writes, writes);

    // The same from the region file in a new session.
    drop(world);
    let mut world = open(&dir, config(0));
    settle(&mut world, eye);
    assert_eq!(world.snapshot().get(target), MaterialId(3));
    assert_eq!(world.stats().edited, 0);
    // Rewriting a voxel with its own material changes nothing either.
    world.apply(&[Edit::new(target, MaterialId(3))]).unwrap();
    assert_eq!(world.stats().edited, 0);
    settle(&mut world, away);
    assert_eq!(world.stats().writes, 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_large_log_is_folded_into_region_files_at_a_checkpoint() {
    let dir = temp_dir("checkpoint");
    let cfg = ResidencyConfig {
        log_checkpoint_kib: 64,
        ..config(0)
    };
    let far = VoxelPos::new(100_000, 0, -100_000);
    let (eye, block, late, far_edit);
    {
        let mut world = open(&dir, cfg.clone());
        eye = eye_at(&world, 2500, 2500);
        settle(&mut world, eye);
        let ground = world.generator().column(far.x, far.z).unwrap().height;
        far_edit = Edit::new(VoxelPos::new(far.x, ground - 1, far.z), MaterialId(3));
        world.apply(&[far_edit]).unwrap();
        // 5200 edits, about 73 KiB of log, in the player's own chunk.
        let cmin = eye_chunk_min(eye);
        block = VoxelBox::from_min_size(cmin, [20, 13, 20]);
        let mut edits = Vec::new();
        for z in 0..20 {
            for y in 0..13 {
                for x in 0..20 {
                    let p = VoxelPos::new(cmin.x + x, cmin.y + y, cmin.z + z);
                    edits.push(Edit::new(p, MaterialId(5)));
                }
            }
        }
        world.apply(&edits).unwrap();
        world.update(eye, FRAME);
        assert!(world.stats().saving >= 1, "the checkpoint writes the chunk");
        // An edit while the checkpoint runs stays in the log.
        late = Edit::new(
            VoxelPos::new(cmin.x + 30, cmin.y + 30, cmin.z + 30),
            MaterialId(6),
        );
        world.apply(&[late]).unwrap();
        world.update(eye, FRAME);
        let s = world.stats();
        assert_eq!(s.checkpoints, 1, "{s:?}");
        assert!(s.log_bytes < 1024, "{s:?}");
        // Then the process "crashes" (no save).
    }
    let log = std::fs::read(dir.join("edits.wvl")).unwrap();
    assert_eq!(
        read_log(&log).unwrap().0,
        vec![
            WalRecord::Checkpoint(1),
            WalRecord::Edits(vec![far_edit]),
            WalRecord::Edits(vec![late]),
        ],
        "the checkpoint, the edit waiting for its chunk, and the late edit"
    );

    let mut world = open(&dir, cfg);
    assert_eq!(world.stats().pending_chunks, 2);
    settle(&mut world, eye);
    let snapshot = world.snapshot();
    assert_eq!(
        snapshot.get(block.min()),
        MaterialId(5),
        "from the region file"
    );
    assert_eq!(snapshot.get(late.pos), MaterialId(6), "from the log");
    assert_eq!(world.stats().pending_chunks, 1, "the far edit still waits");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn replay_starts_at_the_last_checkpoint() {
    let dir = temp_dir("replay_checkpoint");
    std::fs::create_dir_all(&dir).unwrap();
    {
        let (mut wal, _) = WalWriter::open(&dir.join("edits.wvl")).unwrap();
        let a = Edit::new(VoxelPos::new(100_000, 0, -100_000), MaterialId(3));
        let b = Edit::new(VoxelPos::new(-100_000, 0, 100_000), MaterialId(3));
        wal.append(&WalRecord::Edits(vec![a])).unwrap();
        wal.append(&WalRecord::Checkpoint(7)).unwrap();
        wal.append(&WalRecord::Edits(vec![b])).unwrap();
        wal.sync().unwrap();
    }
    let world = open(&dir, config(0));
    assert_eq!(
        world.stats().pending_chunks,
        1,
        "the edit before the checkpoint is in region files already"
    );
    drop(world);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_hot_total_is_kept_without_walking_the_tree() {
    let dir = temp_dir("hot_bytes");
    let mut world = open(&dir, config(0));
    let eye = eye_at(&world, 4000, -2000);
    settle(&mut world, eye);
    let s = world.stats();
    assert!(s.hot_bytes > 0);
    assert_eq!(s.hot_bytes, world.hot_bytes_walked());

    // Edits change it.
    let cmin = eye_chunk_min(eye);
    world
        .fill_box(VoxelBox::from_min_size(cmin, [100, 3, 100]), MaterialId(4))
        .unwrap();
    let target = ground_below(&world, eye);
    dig(&mut world, target);
    assert_eq!(world.stats().hot_bytes, world.hot_bytes_walked());

    // An evicted chunk waiting for its write still counts toward the cap.
    let away = eye_at(&world, eye.x + 200 * 64, eye.z);
    world.update(away, FRAME);
    let s = world.stats();
    assert!(s.saving >= 1 && s.saving_bytes > 0, "{s:?}");
    assert_eq!(s.hot_bytes, world.hot_bytes_walked());
    assert_eq!(world.hot_bytes_counted(), s.hot_bytes + s.saving_bytes);
    settle(&mut world, away);
    assert_eq!(world.stats().saving_bytes, 0);

    // A step aside leaves chunks between the full and the evict radius;
    // over the cap they go.
    let aside = eye_at(&world, away.x + 6 * 64, away.z);
    settle(&mut world, aside);
    let sphere = super::plan::sphere(aside, 16.0).len();
    assert!(world.stats().loaded > sphere, "{:?}", world.stats());
    world.config_mut().hot_cap_mib = 0;
    world.update(aside, FRAME);
    assert_eq!(world.stats().loaded, sphere);
    assert_eq!(world.stats().hot_bytes, world.hot_bytes_walked());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn save_all_keeps_the_log_when_a_write_fails() {
    use world_core::serialize::region::REGION_CHUNKS;
    let dir = temp_dir("write_fails");
    let mut world = open(&dir, config(0));
    let eye = eye_at(&world, -6000, -2500);
    settle(&mut world, eye);
    let target = VoxelPos::new(eye.x, eye.y + 30, eye.z);
    let edit = Edit::new(target, MaterialId(3));
    world.apply(&[edit]).unwrap();
    // A directory where the chunk's region file goes makes its write fail.
    let region = world_gen::ChunkCoord::containing(target)
        .unwrap()
        .to_array()
        .map(|v| v / REGION_CHUNKS);
    let blocker = dir.join(format!("r.{}.{}.{}.wvr", region[0], region[1], region[2]));
    std::fs::create_dir_all(&blocker).unwrap();
    assert!(world.save_all().is_err());
    assert_eq!(world.stats().edited, 1, "still unsaved");
    drop(world);
    let log = std::fs::read(dir.join("edits.wvl")).unwrap();
    assert_eq!(
        read_log(&log).unwrap().0,
        vec![WalRecord::Edits(vec![edit])],
        "the log still holds the edit"
    );

    // Once the region file can be written, the edit comes back from the log.
    std::fs::remove_dir_all(&blocker).unwrap();
    let mut world = open(&dir, config(0));
    assert_eq!(world.stats().pending_chunks, 1);
    settle(&mut world, eye);
    assert_eq!(world.snapshot().get(target), MaterialId(3));
    world.save_all().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A chunk's content as a hash, for comparing loads.
fn chunk_hash(coord: world_gen::ChunkCoord, data: world_core::tree::ChunkData) -> u64 {
    let mut tree = world_core::VoxelTree::unloaded();
    tree.insert_chunk(coord.min_voxel(), data).unwrap();
    world_core::serialize::content_hash(&tree)
}

#[test]
fn unedited_chunks_come_back_from_the_warm_cache_unchanged() {
    let dir = temp_dir("warm_generated");
    let mut world = open(&dir, config(0));
    let eye = eye_at(&world, 3500, 1500);
    settle(&mut world, eye);
    let around = world.loaded_chunks();
    assert_eq!(world.stats().generated, around.len(), "{:?}", world.stats());

    // Away: the unedited chunks are evicted, nothing is written, and they
    // stay compressed in RAM.
    let away = eye_at(&world, eye.x + 200 * 64, eye.z);
    settle(&mut world, away);
    let s = world.stats();
    assert_eq!(s.writes, 0);
    assert!(around.iter().all(|&c| world.is_warm(c)), "{s:?}");
    assert_eq!(s.warm_edited, 0);
    assert_eq!(s.warm_bytes, world.warm_bytes_walked());

    // Back: every one of them is decompressed, none generated again, and
    // each matches the generator exactly.
    let before = world.stats();
    settle(&mut world, eye);
    let after = world.stats();
    assert_eq!(after.warm_hits - before.warm_hits, around.len());
    assert_eq!(after.generated, before.generated);
    for &c in &around {
        let fresh = super::jobs::generate(world.generator(), c);
        let back = world.chunk_data(c).unwrap();
        assert_eq!(chunk_hash(c, back), chunk_hash(c, fresh), "{c:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Digs near (`x`, `z`), then walks away in 40 m steps with a small warm
/// cache. Returns whether the edited chunk is still warm at the end, after
/// checking the warm bytes against the budget at every step.
fn edited_chunk_stays_warm(name: &str, x: i32, z: i32, edited_weight: f32) -> bool {
    let dir = temp_dir(name);
    let cap_mib = 2;
    let mut world = open(
        &dir,
        ResidencyConfig {
            warm_cap_mib: cap_mib,
            warm_edited_weight: edited_weight,
            ..config(0)
        },
    );
    let eye = eye_at(&world, x, z);
    settle(&mut world, eye);
    let target = ground_below(&world, eye);
    dig(&mut world, target);
    let edited = world_gen::ChunkCoord::containing(target).unwrap();
    let mut evicted = 0;
    for step in 1..=4 {
        let at = eye_at(&world, eye.x + step * 40 * 64, eye.z);
        let before = world.stats().loaded;
        settle(&mut world, at);
        evicted += before;
        let s = world.stats();
        assert_eq!(s.warm_bytes, world.warm_bytes_walked(), "{s:?}");
        assert!(s.warm_bytes <= (cap_mib as usize) << 20, "{s:?}");
    }
    let s = world.stats();
    assert!(s.warm_chunks < evicted, "the budget dropped some: {s:?}");
    let warm = world.is_warm(edited);
    drop(world);
    std::fs::remove_dir_all(&dir).unwrap();
    warm
}

#[test]
fn the_warm_cache_keeps_to_its_budget_and_prefers_edited_chunks() {
    assert!(
        !edited_chunk_stays_warm("warm_lru", -3500, 6500, 1.0),
        "plain least recently used: the edited chunk is long gone"
    );
    assert!(
        edited_chunk_stays_warm("warm_weighted", -3500, 6500, 64.0),
        "weighted: it outlasts the generated chunks evicted after it"
    );
}

/// What a walk saw: the chunks joined in order, and the world at the end.
struct Walk {
    joined: Vec<world_gen::ChunkCoord>,
    world: StreamedWorld,
    eye: VoxelPos,
}

/// Opens a world and walks from (`x`, `z`) along +x at 8 m/s for `updates`
/// updates of 1/60 s, zero workers (so at most four loads per update), with
/// the given lookahead.
fn walk(name: &str, x: i32, z: i32, lookahead_secs: f32, updates: i32) -> Walk {
    let dir = temp_dir(name);
    let mut world = open(
        &dir,
        ResidencyConfig {
            full_radius_m: 32.0,
            evict_radius_m: 48.0,
            prefetch_radius_m: 32.0,
            prefetch_lookahead_secs: lookahead_secs,
            velocity_smoothing_secs: 0.0,
            ..config(0)
        },
    );
    let mut joined = Vec::new();
    let mut eye = eye_at(&world, x, z);
    for i in 0..updates {
        // 8 m/s is 512 voxels a second, 8.53 per update.
        eye = eye_at(&world, x + i * 512 / 60, z);
        world.update(eye, FRAME);
        joined.extend(world.take_changes().loaded);
    }
    let _ = std::fs::remove_dir_all(&dir);
    Walk { joined, world, eye }
}

/// Of `chunks`, how many lie ahead of (+x) and behind the chunk at `x`.
fn split_ahead(chunks: &[world_gen::ChunkCoord], x: i32) -> (usize, usize) {
    let at = world_gen::ChunkCoord::containing(VoxelPos::new(x, 0, 0))
        .unwrap()
        .x();
    let ahead = chunks.iter().filter(|c| c.x() > at).count();
    let behind = chunks.iter().filter(|c| c.x() < at).count();
    (ahead, behind)
}

#[test]
fn moving_loads_the_chunks_ahead_first() {
    let (x, z) = (4500, -1500);
    // Starting from nothing and walking at once, the sphere fills over a
    // few dozen updates. The first 40 chunks joined:
    let plain = walk("prefetch_off", x, z, 0.0, 12);
    let ahead = walk("prefetch_on", x, z, 3.0, 12);
    let first = |w: &Walk| w.joined[..40.min(w.joined.len())].to_vec();
    let (p_ahead, p_behind) = split_ahead(&first(&plain), x);
    let (a_ahead, a_behind) = split_ahead(&first(&ahead), x);
    assert!(
        a_ahead > p_ahead && a_behind <= p_behind,
        "prefetch {a_ahead} ahead / {a_behind} behind, nearest first {p_ahead} / {p_behind}"
    );
    assert!(a_ahead > a_behind, "{a_ahead} / {a_behind}");
    // The player's own chunk is never put off.
    let own = world_gen::ChunkCoord::containing(eye_at(&ahead.world, x, z)).unwrap();
    let pos = ahead.joined.iter().position(|&c| c == own).unwrap();
    assert!(pos < 8, "{pos}");

    // The same walk gives the same loads in the same order.
    let again = walk("prefetch_again", x, z, 3.0, 12);
    assert_eq!(again.joined, ahead.joined);
}

#[test]
fn prefetch_loads_ahead_of_the_sphere_and_never_starves_it() {
    let (x, z) = (-5500, 3500);
    let mut w = walk("prefetch_long", x, z, 3.0, 240);
    // Four loads an update keep up at 8 m/s: the sphere around the player is
    // whole, and the lookahead has loaded chunks beyond it, all ahead.
    assert_eq!(w.world.stats().holes, 0, "{:?}", w.world.stats());
    let full = super::plan::voxels(32.0);
    let beyond: Vec<_> = w
        .world
        .loaded_chunks()
        .into_iter()
        .filter(|&c| super::plan::chunk_distance(w.eye, c) > full)
        .collect();
    let (ahead, _) = split_ahead(&beyond, w.eye.x);
    assert!(ahead > 0, "{beyond:?}");
    // Standing still, everything around the player settles.
    settle(&mut w.world, w.eye);
    assert!(w.world.is_settled(w.eye));
}
