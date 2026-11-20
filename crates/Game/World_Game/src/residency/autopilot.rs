//! Developer automation: walks the player camera across the world at a set
//! speed, so a fixed-length run measures streaming under motion (the
//! milestone of decision 0007: a 2 km walk with no frame over 16 ms in
//! mac-floor mode). Nothing inserts [`Autopilot`] in normal play.

use bevy::prelude::*;
use world_core::consts::VOXELS_PER_METER;

use super::{EYE_HEIGHT_M, World};
use crate::player::PlayerCamera;

/// Walks the player camera forward, along the horizontal part of where it
/// looks, at `speed_m_per_s`, keeping it at eye height over the generated
/// terrain. `walked_m` counts the distance covered.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Autopilot {
    pub speed_m_per_s: f32,
    pub walked_m: f32,
}

impl Autopilot {
    pub fn new(speed_m_per_s: f32) -> Self {
        Self {
            speed_m_per_s,
            walked_m: 0.0,
        }
    }
}

/// The horizontal step for `forward` (any length) over `secs`.
fn step(forward: Vec3, speed_m_per_s: f32, secs: f32) -> Vec3 {
    let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
    flat * speed_m_per_s * secs
}

pub(super) fn walk(
    pilot: Option<ResMut<Autopilot>>,
    world: Option<Res<World>>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<PlayerCamera>>,
) {
    let (Some(mut pilot), Some(world)) = (pilot, world) else {
        return;
    };
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let delta = step(*transform.forward(), pilot.speed_m_per_s, time.delta_secs());
    transform.translation += delta;
    pilot.walked_m += delta.length();
    let p = transform.translation * VOXELS_PER_METER as f32;
    if let Ok(column) = world
        .0
        .generator()
        .column(p.x.floor() as i32, p.z.floor() as i32)
    {
        let ground_m = column.height as f32 / VOXELS_PER_METER as f32;
        transform.translation.y = ground_m + EYE_HEIGHT_M;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_horizontal_and_scaled_by_speed_and_time() {
        let d = step(Vec3::new(0.0, -0.5, -1.0), 4.0, 0.5);
        assert_eq!(d.y, 0.0);
        assert!((d.length() - 2.0).abs() < 1e-6);
        assert!(d.z < 0.0);
    }

    #[test]
    fn looking_straight_down_walks_the_default_way() {
        let d = step(Vec3::NEG_Y, 1.0, 1.0);
        assert_eq!(d, Vec3::NEG_Z);
    }
}
