use bevy::prelude::*;

/// Registers World_Anim. It owns no systems yet; each concern registers its own
/// sub-plugin here as it is added.
pub struct WorldAnimPlugin;

impl Plugin for WorldAnimPlugin {
    fn build(&self, _app: &mut App) {}
}
