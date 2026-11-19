//! The local player. For now only the view onto the world exists.

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;

pub(crate) struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera);
    }
}

/// A plain camera so the window has something to present.
/// It becomes the first-person eye camera when the player body exists.
/// Bevy's tonemapping is off: the voxel renderer does its own tone mapping.
fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
    ));
}
