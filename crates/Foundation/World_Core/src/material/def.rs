//! One material definition and the valid range of every field.

use serde::{Deserialize, Serialize};

use super::MaterialId;
use crate::consts::VOXEL_SIZE_M;

/// A colour as three channels in `0.0..=1.0`, sRGB-encoded as artists pick it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgb(pub f32, pub f32, pub f32);

/// What one material is: how it looks, how heavy and strong it is, how it
/// slides and bounces. Units are in the field names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialDef {
    /// Stable id stored in voxels and saves. Never `0` (air), never reused.
    pub id: MaterialId,
    /// Unique, non-empty, human-readable name.
    pub name: String,
    /// Base colour, each channel `0.0..=1.0`.
    pub color: Rgb,
    /// `0.0` mirror-smooth to `1.0` fully rough.
    pub roughness: f32,
    /// Emitted light relative to the sun, `0.0..=100.0`. `0.0` does not glow.
    pub emission: f32,
    /// Kilograms per cubic metre, `1.0..=25_000.0` (air is not a material).
    pub density_kg_m3: f32,
    /// Damage a voxel absorbs before it breaks, `0.0..=1_000.0`. `0.0` breaks on any hit.
    pub strength: f32,
    /// Coulomb friction coefficient, `0.0..=2.0`.
    pub friction: f32,
    /// Bounciness, `0.0..=1.0`.
    pub restitution: f32,
    /// Sound bank key played when a voxel of this material breaks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_sound: Option<String>,
    /// Sound bank key played on impacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact_sound: Option<String>,
}

impl MaterialDef {
    /// Mass of one render voxel of this material, in kilograms.
    pub fn mass_per_voxel_kg(&self) -> f32 {
        self.density_kg_m3 * VOXEL_SIZE_M * VOXEL_SIZE_M * VOXEL_SIZE_M
    }

    /// Every out-of-range field, as `(field, problem)` pairs.
    pub(super) fn range_problems(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        let mut check = |field: &'static str, value: f32, lo: f32, hi: f32| {
            if !(lo..=hi).contains(&value) {
                out.push((field, format!("{value} is outside {lo}..={hi}")));
            }
        };
        check("color.0", self.color.0, 0.0, 1.0);
        check("color.1", self.color.1, 0.0, 1.0);
        check("color.2", self.color.2, 0.0, 1.0);
        check("roughness", self.roughness, 0.0, 1.0);
        check("emission", self.emission, 0.0, 100.0);
        check("density_kg_m3", self.density_kg_m3, 1.0, 25_000.0);
        check("strength", self.strength, 0.0, 1_000.0);
        check("friction", self.friction, 0.0, 2.0);
        check("restitution", self.restitution, 0.0, 1.0);
        if self.name.trim().is_empty() {
            out.push(("name", "is empty".into()));
        }
        if self.id.is_air() {
            out.push(("id", "0 is reserved for air".into()));
        }
        out
    }
}

#[cfg(test)]
pub(crate) fn sample(id: u16, name: &str) -> MaterialDef {
    MaterialDef {
        id: MaterialId(id),
        name: name.into(),
        color: Rgb(0.5, 0.5, 0.5),
        roughness: 0.8,
        emission: 0.0,
        density_kg_m3: 2_400.0,
        strength: 50.0,
        friction: 0.7,
        restitution: 0.1,
        break_sound: None,
        impact_sound: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sensible_material_has_no_problems() {
        assert!(sample(1, "stone").range_problems().is_empty());
    }

    #[test]
    fn nan_is_out_of_every_range() {
        let mut m = sample(1, "stone");
        m.friction = f32::NAN;
        let problems = m.range_problems();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, "friction");
    }

    #[test]
    fn air_and_blank_names_are_rejected() {
        let m = sample(0, "  ");
        let fields: Vec<_> = m.range_problems().into_iter().map(|p| p.0).collect();
        assert_eq!(fields, ["name", "id"]);
    }

    #[test]
    fn one_voxel_of_water_weighs_about_four_grams() {
        let mut m = sample(1, "water");
        m.density_kg_m3 = 1_000.0;
        let grams = m.mass_per_voxel_kg() * 1_000.0;
        assert!((grams - 3.8147).abs() < 1e-3, "{grams}");
    }
}
