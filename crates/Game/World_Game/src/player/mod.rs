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
///
/// MSAA is off. The voxels arrive as one fullscreen triangle, so every
/// sample of a pixel gets the same colour and depth and the resolve changes
/// nothing; Bevy's default of four samples only multiplied the composite's
/// writes at the window's size and added a resolve (decision 0017).
fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        PlayerCamera,
        ReloadScoped,
        Camera3d::default(),
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
        SkyComposite::default(),
    ));
}
