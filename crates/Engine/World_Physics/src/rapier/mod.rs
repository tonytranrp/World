//! The only module that names Rapier.

use bevy::prelude::*;
use bevy_rapier3d::prelude::{NoUserData, RapierPhysicsPlugin};

pub(crate) fn build(app: &mut App) {
    app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule());
}
