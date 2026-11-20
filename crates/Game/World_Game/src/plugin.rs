use bevy::prelude::*;

use crate::interaction::InteractionPlugin;
use crate::player::PlayerPlugin;
use crate::residency::ResidencyPlugin;
use crate::ui::UiPlugin;

/// Registers all gameplay domains.
pub struct WorldGamePlugin;

impl Plugin for WorldGamePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((PlayerPlugin, ResidencyPlugin, InteractionPlugin, UiPlugin));
    }
}
