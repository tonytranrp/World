//! The render passes, in frame order: primary rays, then composite.

pub mod composite;
pub mod primary;

use bevy::prelude::*;

pub(crate) fn build(app: &mut App) {
    composite::build(app);
}
