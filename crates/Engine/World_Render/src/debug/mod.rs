//! Debugging aids: the built-in test scene, fed to the renderer the way the
//! game will feed it a world.

pub mod scene;

use std::sync::Arc;

use bevy::prelude::*;
use world_core::VoxelTree;
use world_reload::Config;

use crate::brick_cache::config::BrickCacheConfig;
use crate::brick_cache::stream::feed::VoxelWorldFeed;

/// The scene while it is what the renderer shows.
#[derive(Resource)]
pub struct DebugScene {
    tree: VoxelTree,
    frame: u32,
}

/// Publishes the scene at startup when the config asks for it and nothing
/// else has fed the renderer a world.
pub fn start_debug_scene(
    mut commands: Commands,
    config: Res<Config<BrickCacheConfig>>,
    mut feed: ResMut<VoxelWorldFeed>,
) {
    if !config.debug_scene || feed.snapshot().is_some() {
        return;
    }
    let tree = scene::build();
    feed.replace(Arc::new(tree.snapshot()));
    commands.insert_resource(DebugScene { tree, frame: 0 });
    info!("showing the built-in debug voxel scene (brick_cache.ron: debug_scene)");
}

/// Animates the scene's edits, or withdraws it when the config turns it off.
pub fn update_debug_scene(
    mut commands: Commands,
    config: Res<Config<BrickCacheConfig>>,
    scene: Option<ResMut<DebugScene>>,
    mut feed: ResMut<VoxelWorldFeed>,
) {
    let Some(mut scene) = scene else {
        return;
    };
    if !config.debug_scene {
        feed.clear();
        commands.remove_resource::<DebugScene>();
        return;
    }
    scene.frame += 1;
    if config.debug_scene_edits {
        let frame = scene.frame;
        let dirty = scene::animate(&mut scene.tree, frame);
        if !dirty.is_empty() {
            feed.publish(Arc::new(scene.tree.snapshot()), &dirty);
        }
    }
}
