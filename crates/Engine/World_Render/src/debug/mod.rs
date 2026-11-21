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
    /// The feed's generation once the scene was published: a different one
    /// means a real world replaced the scene.
    generation: u64,
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
    commands.insert_resource(DebugScene {
        tree,
        frame: 0,
        generation: feed.generation(),
    });
    info!("showing the built-in debug voxel scene (brick_cache.ron: debug_scene)");
}

/// Animates the scene's edits, or withdraws it when the config turns it off
/// or a real world replaced it. A withdrawn scene never publishes again, so
/// its edits cannot reach the world that took its place.
pub fn update_debug_scene(
    mut commands: Commands,
    config: Res<Config<BrickCacheConfig>>,
    scene: Option<ResMut<DebugScene>>,
    mut feed: ResMut<VoxelWorldFeed>,
) {
    let Some(mut scene) = scene else {
        return;
    };
    if feed.generation() != scene.generation {
        commands.remove_resource::<DebugScene>();
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use world_core::{MaterialId, VoxelBox, VoxelPos};

    fn world_with_scene() -> World {
        let mut world = World::new();
        world.insert_resource(Config::<BrickCacheConfig>::default());
        world.init_resource::<VoxelWorldFeed>();
        world.run_system_once(start_debug_scene).unwrap();
        assert!(world.contains_resource::<DebugScene>());
        world
    }

    #[test]
    fn the_scene_animates_while_it_is_shown() {
        let mut world = world_with_scene();
        let _ = world.resource_mut::<VoxelWorldFeed>().take();
        for _ in 0..4 {
            world.run_system_once(update_debug_scene).unwrap();
        }
        assert!(
            !world
                .resource_mut::<VoxelWorldFeed>()
                .take()
                .dirty
                .is_empty()
        );
    }

    #[test]
    fn a_real_world_replacing_the_scene_stops_its_edits() {
        let mut world = world_with_scene();
        let mut real = VoxelTree::new();
        real.fill_box(
            VoxelBox::new(VoxelPos::new(0, 0, 0), VoxelPos::new(16, 16, 16)),
            MaterialId(2),
        )
        .unwrap();
        let snapshot = Arc::new(real.snapshot());
        world
            .resource_mut::<VoxelWorldFeed>()
            .replace(snapshot.clone());
        let _ = world.resource_mut::<VoxelWorldFeed>().take();

        for _ in 0..16 {
            world.run_system_once(update_debug_scene).unwrap();
            let frame = world.resource_mut::<VoxelWorldFeed>().take();
            assert!(
                frame.dirty.is_empty(),
                "the withdrawn scene published edits"
            );
            assert!(!frame.refresh, "withdrawing must not clear the real world");
            assert!(Arc::ptr_eq(frame.snapshot.as_ref().unwrap(), &snapshot));
        }
        assert!(!world.contains_resource::<DebugScene>());
    }
}
