//! The local player. For now only the view onto the world exists.

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use world_reload::{AppReloadExt, ReloadScoped};
use world_render::prelude::SkyComposite;

pub(crate) struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        // Reloadable: after a code hot-patch the camera is rebuilt with the new code.
        app.add_reloadable_systems(|| spawn_camera);
    }
}

/// The camera the player looks through.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PlayerCamera;

/// A plain camera so the window has something to present, with the sky
/// composited over it. It becomes the first-person eye camera when the player
/// body exists. Bevy's tonemapping is off: the voxel renderer does its own.
fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        PlayerCamera,
        ReloadScoped,
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        SkyComposite::default(),
    ));
}
