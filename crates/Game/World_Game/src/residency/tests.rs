//! Residency end to end, without Bevy: loading around the player, edits
//! surviving eviction, reload and a crash, edits to unloaded chunks, the
//! coarse tier and the threaded pool (decision 0006).

use std::path::{Path, PathBuf};

use world_core::consts::VOXELS_PER_METER;
use world_core::serialize::wal::read_log;
use world_core::{Edit, MaterialId, VoxelBox, VoxelPos};
use world_gen::TerrainConfig;

use super::config::ResidencyConfig;
use super::world::StreamedWorld;

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
        world.update(eye);
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
        world.update(eye);
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
        world.update(eye);
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
    let loaded = jobs::load(world.generator(), 1, coord, None, &saved);
    assert!(!loaded.edited, "the generated chunk stands in");
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
        world.update(eye);
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
        world.update(eye);
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
