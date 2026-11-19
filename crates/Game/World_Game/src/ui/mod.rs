//! Everything the player reads or clicks that is not the world. So far: the
//! developer panels, which exist only with the `dev_tools` feature.

#[cfg(feature = "dev_tools")]
mod dev;

use bevy::prelude::*;

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, _app: &mut App) {
        #[cfg(feature = "dev_tools")]
        _app.add_plugins(dev::DevToolsPlugin);
    }
}
