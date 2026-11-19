use bevy::prelude::*;

use crate::{code, config};

/// Live reloading of configs and code.
///
/// Configs work without this plugin (`add_config` sets up what it needs), but
/// the code-reload path and the config file root for saving come from here.
#[derive(Default)]
pub struct WorldReloadPlugin;

impl Plugin for WorldReloadPlugin {
    fn build(&self, app: &mut App) {
        config::build(app);
        code::build(app);
    }
}
