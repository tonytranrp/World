//! The render passes, in frame order: primary rays, lighting, then
//! composite.

pub mod composite;
pub mod lighting;
pub mod primary;

use bevy::prelude::*;

pub(crate) fn build(app: &mut App) {
    composite::build(app);
}
