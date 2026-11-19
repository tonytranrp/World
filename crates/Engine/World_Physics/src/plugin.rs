use bevy::prelude::*;
use world_reload::AppConfigExt;

use crate::rapier;
use crate::tuning::{PHYSICS_CONFIG_PATH, PhysicsTuning};

/// Registers physics. Runs in the fixed schedule at the rate set by
/// `assets/config/physics.ron` (60 Hz by default), which hot-reloads.
pub struct WorldPhysicsPlugin;

impl Plugin for WorldPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_config::<PhysicsTuning>(PHYSICS_CONFIG_PATH);
        rapier::build(app);
    }
}
