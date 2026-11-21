//! Residency: which chunks of the world are in memory, in which form, and
//! how every edit is kept (decision 0006). The core is plain Rust in
//! [`world`]; this module adds the Bevy glue.

mod autopilot;
mod columns;
pub mod config;
mod jobs;
pub mod plan;
mod pool;
mod store;
pub mod world;

#[cfg(test)]
mod tests;
mod warm;

use std::sync::Arc;

use bevy::prelude::*;
use world_core::consts::VOXELS_PER_METER;
use world_core::{ConfigAsset, DirtyBricks, VoxelPos};
use world_gen::TerrainConfig;
use world_reload::{AppConfigExt, Config, ConfigOrigin, ConfigRegistry};
use world_render::prelude::VoxelWorldFeed;

pub use autopilot::Autopilot;
pub use config::ResidencyConfig;
use world::ResidencyStats;
pub use world::StreamedWorld;

use crate::player::PlayerCamera;

/// The world the game plays in, streamed around the player camera.
#[derive(Resource)]
pub struct World(pub StreamedWorld);

pub(crate) struct ResidencyPlugin;

impl Plugin for ResidencyPlugin {
    fn build(&self, app: &mut App) {
        app.add_config::<ResidencyConfig>()
            .add_config::<TerrainConfig>()
            .add_systems(
                Update,
                (
                    open_world,
                    place_on_ground,
                    autopilot::walk,
                    update_world,
                    feed_renderer,
                    log_stats,
                )
                    .chain(),
            )
            .add_systems(Last, save_on_exit);
    }
}

/// The voxel the camera is in. Bevy units are metres.
fn eye_voxel(transform: &GlobalTransform) -> VoxelPos {
    let p = transform.translation() * VOXELS_PER_METER as f32;
    VoxelPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
}

/// How long to wait for the config files before opening the world with
/// the safe defaults (a missing or broken file never blocks the game).
const CONFIG_GRACE_SECS: f32 = 3.0;

/// Opens the world once the config files are in, and again when either
/// changes, so the seed and terrain can be tuned live (unsaved edits are
/// saved first).
fn open_world(
    mut commands: Commands,
    world: Option<ResMut<World>>,
    residency: Res<Config<ResidencyConfig>>,
    terrain: Res<Config<TerrainConfig>>,
    registry: Res<ConfigRegistry>,
    time: Res<Time>,
) {
    if world.is_some() && !residency.is_changed() && !terrain.is_changed() {
        return;
    }
    let from_file = |name: &str| {
        registry
            .get(name)
            .is_some_and(|e| e.origin != ConfigOrigin::Defaults)
    };
    let files_in = from_file(ResidencyConfig::NAME) && from_file(TerrainConfig::NAME);
    if world.is_none() && !files_in && time.elapsed_secs() < CONFIG_GRACE_SECS {
        return;
    }
    if let Some(mut old) = world
        && let Err(e) = old.0.save_all()
    {
        error!("saving the world before reopening it: {e}");
    }
    let dir = std::path::PathBuf::from(&residency.save_dir);
    match StreamedWorld::open((**residency).clone(), (**terrain).clone(), &dir) {
        Ok(streamed) => {
            info!(
                "world opened: seed {:#x}, saves in {}",
                residency.seed,
                dir.display()
            );
            commands.insert_resource(World(streamed));
        }
        Err(e) => error!("cannot open the world in {}: {e}", dir.display()),
    }
}

fn update_world(
    world: Option<ResMut<World>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    time: Res<Time>,
    mut opened_at: Local<Option<f32>>,
) {
    let Some(mut world) = world else {
        return;
    };
    let Ok(transform) = camera.single() else {
        return;
    };
    if world.is_added() {
        *opened_at = Some(time.elapsed_secs());
    }
    let eye = eye_voxel(transform);
    world.0.update(eye, time.delta_secs());
    if let Some(start) = *opened_at
        && world.0.is_settled(eye)
    {
        let s = world.0.stats();
        info!(
            "world settled around the player in {:.2} s: {} chunks, {:.0} MiB",
            time.elapsed_secs() - start,
            s.loaded,
            s.hot_bytes as f64 / 1048576.0
        );
        *opened_at = None;
    }
    for e in world.0.take_errors() {
        warn!("world residency: {e}");
    }
}

/// Hands the renderer this frame's render tree and the bricks that changed,
/// with the summaries the workers computed for the chunks it replaced. A
/// newly opened world replaces whatever the renderer showed.
fn feed_renderer(world: Option<ResMut<World>>, feed: Option<ResMut<VoxelWorldFeed>>) {
    let (Some(mut world), Some(mut feed)) = (world, feed) else {
        return;
    };
    let snapshot = Arc::new(world.0.render_snapshot());
    let changes = world.0.take_changes();
    let summaries = world.0.take_render_summaries();
    if world.is_added() {
        feed.replace(snapshot);
    } else {
        feed.publish_with_chunks(
            snapshot,
            &DirtyBricks::from_unsorted(changes.render_bricks),
            &changes.render_chunks,
        );
        feed.offer_summaries(summaries);
    }
}

/// Puts the camera at eye height above the terrain once per opened world,
/// where the player stands.
/// The player's eye above the ground, in metres.
const EYE_HEIGHT_M: f32 = 1.7;

fn place_on_ground(
    world: Option<Res<World>>,
    mut camera: Query<&mut Transform, With<PlayerCamera>>,
    mut placed: Local<bool>,
) {
    let Some(world) = world else {
        return;
    };
    if world.is_added() {
        *placed = false;
    }
    if *placed {
        return;
    }
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let p = transform.translation * VOXELS_PER_METER as f32;
    let Ok(column) = world
        .0
        .generator()
        .column(p.x.floor() as i32, p.z.floor() as i32)
    else {
        return;
    };
    let ground_m = column.height as f32 / VOXELS_PER_METER as f32;
    transform.translation.y = ground_m + EYE_HEIGHT_M;
    *placed = true;
}

/// Logs the residency figures every few seconds: what is held, and over the
/// interval the mean number of holes in the full-detail sphere (all of them
/// and those ahead of the player's motion) and where loaded chunks came from.
fn log_stats(world: Option<Res<World>>, time: Res<Time>, mut last: Local<(f32, ResidencyStats)>) {
    let Some(world) = world else {
        return;
    };
    if world.is_added() {
        *last = (time.elapsed_secs(), ResidencyStats::default());
    }
    let now = time.elapsed_secs();
    if now - last.0 < 5.0 {
        return;
    }
    let s = world.0.stats();
    let p = std::mem::replace(&mut *last, (now, s)).1;
    let updates = s.updates.saturating_sub(p.updates).max(1) as f64;
    let mean = |sum: u64, before: u64| sum.saturating_sub(before) as f64 / updates;
    let v = world.0.velocity_m_s();
    debug!(
        "world: {} chunks loaded ({} edited, {} loading, {} saving, {:.0} MiB held), {:.0} MiB hot, \
         {} joining ({:.0} MiB), {} compressing ({:.0} MiB), {} warm ({} edited, {:.0} MiB), {} coarse columns ({:.0} MiB), {} saved, {} waiting; \
         interval: holes {:.1} mean ({:.1} ahead), {} now, loads {} generated / {} warm / \
         {} disk, generation {:.1} ms mean per chunk (longest {:.0} ms so far, {} uniform), columns {} hits / {} \
         computed ({:.0} MiB held), speed {:.1} m/s",
        s.loaded,
        s.edited,
        s.loading,
        s.saving,
        s.saving_bytes as f64 / 1048576.0,
        s.hot_bytes as f64 / 1048576.0,
        s.ready,
        s.ready_bytes as f64 / 1048576.0,
        s.compressing,
        s.compressing_bytes as f64 / 1048576.0,
        s.warm_chunks,
        s.warm_edited,
        s.warm_bytes as f64 / 1048576.0,
        s.coarse_columns,
        s.coarse_bytes as f64 / 1048576.0,
        s.saved_chunks,
        s.pending_chunks,
        mean(s.hole_sum, p.hole_sum),
        mean(s.hole_ahead_sum, p.hole_ahead_sum),
        s.holes,
        s.generated - p.generated.min(s.generated),
        s.warm_hits - p.warm_hits.min(s.warm_hits),
        s.disk_loads - p.disk_loads.min(s.disk_loads),
        {
            let n = s.generated.saturating_sub(p.generated).max(1) as f64;
            s.generated_micros.saturating_sub(p.generated_micros) as f64 / n / 1000.0
        },
        s.generated_max_micros as f64 / 1000.0,
        s.uniform_loads - p.uniform_loads.min(s.uniform_loads),
        s.column_hits - p.column_hits.min(s.column_hits),
        s.column_misses - p.column_misses.min(s.column_misses),
        s.column_bytes as f64 / 1048576.0,
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(),
    );
}

fn save_on_exit(mut exits: MessageReader<AppExit>, world: Option<ResMut<World>>) {
    if exits.read().next().is_none() {
        return;
    }
    if let Some(mut world) = world {
        match world.0.save_all() {
            Ok(()) => info!("world saved"),
            Err(e) => error!("saving the world on exit: {e}"),
        }
    }
}
