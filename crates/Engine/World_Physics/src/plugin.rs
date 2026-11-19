use bevy::prelude::*;

use crate::rapier;

/// Registers physics. Runs at a fixed 60 Hz in the fixed schedule.
pub struct WorldPhysicsPlugin;

impl Plugin for WorldPhysicsPlugin {
    fn build(&self, app: &mut App) {
        rapier::build(app);
    }
}
