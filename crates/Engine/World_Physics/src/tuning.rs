//! Physics tuning, loaded from `assets/config/physics.ron` and hot-reloaded.
//!
//! Engine-neutral numbers only: the `rapier` module translates them for Rapier.

use serde::{Deserialize, Serialize};
use world_core::ConfigAsset;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhysicsTuning {
    /// Fixed physics steps per second, `30.0..=240.0`.
    pub step_hz: f64,
    /// Gravity in metres per second squared. Magnitude at most `100.0`.
    pub gravity_m_s2: (f32, f32, f32),
    /// Not read yet: destruction does not exist. Validated so the file is
    /// ready when it does.
    pub debris: DebrisTuning,
    /// Not read yet: the character controller does not exist.
    pub character: CharacterTuning,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DebrisTuning {
    /// Islands with at most this many voxels become particles, not bodies. `1..=4096`.
    pub particle_max_voxels: u32,
    /// Hard cap on awake rigid bodies. `1..=10_000`.
    pub max_active_bodies: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterTuning {
    /// Highest ledge the controller climbs without jumping, metres. `0.0..=1.0`.
    pub step_up_m: f32,
    /// Distance the controller snaps down to stay on the ground, metres. `0.0..=1.0`.
    pub snap_to_ground_m: f32,
    /// Steepest walkable slope, degrees. `0.0..=89.0`.
    pub max_slope_deg: f32,
}

impl Default for PhysicsTuning {
    fn default() -> Self {
        Self {
            step_hz: 60.0,
            gravity_m_s2: (0.0, -9.81, 0.0),
            debris: DebrisTuning {
                particle_max_voxels: 64,
                max_active_bodies: 500,
            },
            character: CharacterTuning {
                step_up_m: 0.3,
                snap_to_ground_m: 0.2,
                max_slope_deg: 50.0,
            },
        }
    }
}

impl ConfigAsset for PhysicsTuning {
    const NAME: &'static str = "physics";
    const PATH: &'static str = "config/physics.ron";

    fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut range = |field: &str, value: f64, lo: f64, hi: f64| {
            if !(lo..=hi).contains(&value) {
                problems.push(format!("{field} = {value} is outside {lo}..={hi}"));
            }
        };
        range("step_hz", self.step_hz, 30.0, 240.0);
        let (x, y, z) = self.gravity_m_s2;
        let magnitude = f64::from(x * x + y * y + z * z).sqrt();
        range("gravity_m_s2 magnitude", magnitude, 0.0, 100.0);
        range(
            "debris.particle_max_voxels",
            self.debris.particle_max_voxels.into(),
            1.0,
            4096.0,
        );
        range(
            "debris.max_active_bodies",
            self.debris.max_active_bodies.into(),
            1.0,
            10_000.0,
        );
        range(
            "character.step_up_m",
            self.character.step_up_m.into(),
            0.0,
            1.0,
        );
        range(
            "character.snap_to_ground_m",
            self.character.snap_to_ground_m.into(),
            0.0,
            1.0,
        );
        range(
            "character.max_slope_deg",
            self.character.max_slope_deg.into(),
            0.0,
            89.0,
        );
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        assert_eq!(PhysicsTuning::default().validate(), Ok(()));
    }

    #[test]
    fn every_bad_field_is_reported() {
        let mut t = PhysicsTuning {
            step_hz: 1.0,
            gravity_m_s2: (0.0, f32::NAN, 0.0),
            ..PhysicsTuning::default()
        };
        t.character.max_slope_deg = 95.0;
        let problems = t.validate().unwrap_err();
        assert_eq!(problems.len(), 3, "{problems:?}");
    }
}
