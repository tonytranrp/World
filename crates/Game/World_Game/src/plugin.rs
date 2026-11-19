use bevy::prelude::*;

use crate::player::PlayerPlugin;

/// Registers all gameplay domains.
pub struct WorldGamePlugin;

impl Plugin for WorldGamePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PlayerPlugin);
    }
}
