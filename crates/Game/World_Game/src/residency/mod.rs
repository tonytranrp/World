//! Residency: which chunks of the world are in memory, in which form, and
//! how every edit is kept (decision 0006). The core is plain Rust in
//! [`world`]; this module adds the Bevy glue.

pub mod config;
mod jobs;
pub mod plan;
mod pool;
mod store;
pub mod world;

#[cfg(test)]
mod tests;

use bevy::prelude::*;
use world_core::VoxelPos;
use world_core::consts::VOXELS_PER_METER;
use world_gen::TerrainConfig;
use world_reload::{AppConfigExt, Config};

pub use config::ResidencyConfig;
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
                (open_world, place_on_ground, update_world, log_stats).chain(),
            )
            .add_systems(Last, save_on_exit);
    }
}

/// The voxel the camera is in. Bevy units are metres.
fn eye_voxel(transform: &GlobalTransform) -> VoxelPos {
    let p = transform.translation() * VOXELS_PER_METER as f32;
    VoxelPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
}

/// Opens the world once the configs are in, and again when either changes,
/// so the seed and terrain can be tuned live (unsaved edits are saved first).
fn open_world(
    mut commands: Commands,
    world: Option<ResMut<World>>,
    residency: Res<Config<ResidencyConfig>>,
    terrain: Res<Config<TerrainConfig>>,
) {
    if world.is_some() && !residency.is_changed() && !terrain.is_changed() {
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

fn update_world(world: Option<ResMut<World>>, camera: Query<&GlobalTransform, With<PlayerCamera>>) {
    let Some(mut world) = world else {
        return;
    };
    let Ok(transform) = camera.single() else {
        return;
    };
    world.0.update(eye_voxel(transform));
    for e in world.0.take_errors() {
        warn!("world residency: {e}");
    }
}

/// Lifts the camera above the terrain once per opened world, if it starts
/// underground.
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
    if transform.translation.y < ground_m + 1.8 {
        transform.translation.y = ground_m + 1.8;
    }
    *placed = true;
}

/// Logs the residency figures every few seconds.
fn log_stats(world: Option<Res<World>>, time: Res<Time>, mut last: Local<f32>) {
    let Some(world) = world else {
        return;
    };
    let now = time.elapsed_secs();
    if now - *last < 5.0 {
        return;
    }
    *last = now;
    let s = world.0.stats();
    debug!(
        "world: {} chunks loaded ({} edited, {} loading, {} saving), {:.0} MiB hot,          {} warm ({:.0} MiB), {} coarse columns ({:.0} MiB), {} saved, {} waiting",
        s.loaded,
        s.edited,
        s.loading,
        s.saving,
        s.hot_bytes as f64 / 1048576.0,
        s.warm_chunks,
        s.warm_bytes as f64 / 1048576.0,
        s.coarse_columns,
        s.coarse_bytes as f64 / 1048576.0,
        s.saved_chunks,
        s.pending_chunks,
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
