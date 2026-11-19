//! The render passes, in frame order. Only `composite` exists so far.

pub mod composite;

use bevy::prelude::*;

pub(crate) fn build(app: &mut App) {
    composite::build(app);
}
