use bevy::prelude::*;

/// Registers World_Sim. It owns no systems yet; each concern registers its own
/// sub-plugin here as it is added.
pub struct WorldSimPlugin;

impl Plugin for WorldSimPlugin {
    fn build(&self, _app: &mut App) {}
}
