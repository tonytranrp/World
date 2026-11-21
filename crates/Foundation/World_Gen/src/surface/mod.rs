//! Surface detail: what each solid voxel of a column is made of.
//!
//! A column is a height and three layers from the top down: the surface cover
//! (`surface_depth` voxels of the surface material), soil (dirt down to
//! `soil_depth` voxels below the top), and bedrock (stone) beneath. The rules
//! pick the surface by slope, altitude and moisture:
//!
//! 1. slope at or above `cliff_slope`, or altitude at or above
//!    `rock_line_m`: bare stone, no soil;
//! 2. altitude below `sea_level_m`, moisture below `dry_moisture`, or slope at
//!    or above `grass_max_slope`: bare dirt over stone;
//! 3. otherwise grass, 2 to 5 voxels thick by a per-column hash, over dirt
//!    over stone.
//!
//! Every rule has a result, so no solid voxel is ever without a material.
//!
//! Materials are `world_core::MaterialId`s matching `assets/config/materials.ron`
//! by name: stone is 1, dirt 2 and grass 3 (the defaults of
//! [`SurfaceParams`]). [`SurfaceParams::check_materials`] confirms the ids
//! against a loaded table.

use serde::{Deserialize, Serialize};

use crate::fingerprint::Fingerprint;
use crate::noise::{derive, hash_column};
use crate::regional::ColumnInputs;
use world_core::consts::VOXELS_PER_METER;
use world_core::{MaterialId, MaterialTable};

/// The surface parameters, from the terrain config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceParams {
    /// Surface cover of gentle, moist land: `grass` in the material table.
    pub grass: MaterialId,
    /// Soil, and bare ground where grass does not grow: `dirt`.
    pub dirt: MaterialId,
    /// Bedrock, cliffs and high ground: `stone`.
    pub stone: MaterialId,
    /// Altitude of the sea surface, in metres. There is no water in
    /// version 1; ground below it is bare dirt.
    pub sea_level_m: f32,
    /// Altitude at and above which the ground is bare stone, in metres.
    pub rock_line_m: f32,
    /// Slope (rise over run) at and above which grass gives way to dirt.
    pub grass_max_slope: f32,
    /// Slope at and above which the ground is bare stone.
    pub cliff_slope: f32,
    /// Moisture (`-1` to `1`) below which the ground is bare dirt.
    pub dry_moisture: f32,
    /// Thinnest and thickest grass layer, in voxels (inclusive).
    pub grass_depth_voxels: [u8; 2],
}

impl Default for SurfaceParams {
    fn default() -> Self {
        Self {
            grass: MaterialId(3),
            dirt: MaterialId(2),
            stone: MaterialId(1),
            sea_level_m: 0.0,
            rock_line_m: 150.0,
            grass_max_slope: 0.8,
            cliff_slope: 1.4,
            dry_moisture: -0.3,
            grass_depth_voxels: [2, 5],
        }
    }
}

impl SurfaceParams {
    /// Every problem with these parameters on their own.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, id) in [
            ("grass", self.grass),
            ("dirt", self.dirt),
            ("stone", self.stone),
        ] {
            if id.is_air() {
                out.push(format!("surface.{name} must not be air (0)"));
            }
        }
        for (name, v) in [
            ("sea_level_m", self.sea_level_m),
            ("rock_line_m", self.rock_line_m),
            ("dry_moisture", self.dry_moisture),
        ] {
            if !v.is_finite() {
                out.push(format!("surface.{name} {v} must be finite"));
            }
        }
        if !(self.grass_max_slope.is_finite()
            && self.cliff_slope.is_finite()
            && 0.0 < self.grass_max_slope
            && self.grass_max_slope <= self.cliff_slope)
        {
            out.push(format!(
                "surface: need 0 < grass_max_slope ({}) <= cliff_slope ({})",
                self.grass_max_slope, self.cliff_slope
            ));
        }
        let [thin, thick] = self.grass_depth_voxels;
        if !(1 <= thin && thin <= thick) {
            out.push(format!(
                "surface.grass_depth_voxels {:?} must be 1 <= thinnest <= thickest",
                self.grass_depth_voxels
            ));
        }
        out
    }

    /// Every material id that `table` does not define, so a config that names
    /// a missing material is rejected rather than guessed.
    pub fn check_materials(&self, table: &MaterialTable) -> Result<(), Vec<String>> {
        let missing: Vec<String> = [
            ("grass", self.grass),
            ("dirt", self.dirt),
            ("stone", self.stone),
        ]
        .into_iter()
        .filter(|(_, id)| table.get(*id).is_none())
        .map(|(name, id)| {
            format!(
                "surface.{name} is material {}, which the table does not define",
                id.0
            )
        })
        .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    pub(crate) fn fingerprint(&self, f: &mut Fingerprint) {
        f.u32(self.grass.0.into())
            .u32(self.dirt.0.into())
            .u32(self.stone.0.into())
            .f32(self.sea_level_m)
            .f32(self.rock_line_m)
            .f32(self.grass_max_slope)
            .f32(self.cliff_slope)
            .f32(self.dry_moisture)
            .u32(self.grass_depth_voxels[0].into())
            .u32(self.grass_depth_voxels[1].into());
    }
}

/// The materials beneath a column's surface cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layers {
    /// Between the surface cover and `soil_depth`.
    pub soil: MaterialId,
    /// Everything deeper.
    pub bedrock: MaterialId,
}

/// One generated column. Eight bytes as a value; a chunk footprint
/// ([`ChunkColumns`](crate::ChunkColumns)) stores its columns packed in four.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Column {
    /// Voxels with `y < height` are solid; the top solid voxel is
    /// `height - 1`.
    pub height: i32,
    /// The top material: grass, dirt or stone.
    pub surface: MaterialId,
    /// Voxels of `surface` from the top down (0 when the top is bare soil or
    /// stone).
    pub surface_depth: u8,
    /// Depth below the top where soil ends and bedrock begins (0 for bare
    /// stone).
    pub soil_depth: u8,
}

impl Column {
    /// The material of voxel `y` of this column: air at and above `height`,
    /// then the surface cover, soil and bedrock by depth below the top.
    #[inline]
    pub fn material_at(&self, y: i32, layers: &Layers) -> MaterialId {
        if y >= self.height {
            return MaterialId::AIR;
        }
        let depth = i64::from(self.height) - 1 - i64::from(y);
        if depth < i64::from(self.surface_depth) {
            self.surface
        } else if depth < i64::from(self.soil_depth) {
            layers.soil
        } else {
            layers.bedrock
        }
    }

    /// Every voxel with `y` below this is bedrock, so a builder can fill
    /// whole subtrees under it without asking per voxel.
    pub fn bedrock_below(&self) -> i32 {
        let layer = i64::from(self.surface_depth.max(self.soil_depth));
        (i64::from(self.height) - layer).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }
}

/// The surface rules of one seed, with lengths converted to voxels.
#[derive(Clone)]
pub struct Surface {
    seed: u64,
    grass: MaterialId,
    layers: Layers,
    sea_level: f32,
    rock_line: f32,
    grass_max_slope: f32,
    cliff_slope: f32,
    dry_moisture: f32,
    grass_thin: u8,
    grass_choices: u64,
}

impl Surface {
    pub fn new(seed: u64, params: &SurfaceParams) -> Self {
        let vpm = VOXELS_PER_METER as f32;
        let [thin, thick] = params.grass_depth_voxels;
        Self {
            seed: derive(seed, "surface.grass_depth"),
            grass: params.grass,
            layers: Layers {
                soil: params.dirt,
                bedrock: params.stone,
            },
            sea_level: params.sea_level_m * vpm,
            rock_line: params.rock_line_m * vpm,
            grass_max_slope: params.grass_max_slope,
            cliff_slope: params.cliff_slope,
            dry_moisture: params.dry_moisture,
            grass_thin: thin,
            grass_choices: u64::from(thick.saturating_sub(thin)) + 1,
        }
    }

    pub fn layers(&self) -> Layers {
        self.layers
    }

    /// Every surface material a column can have: bedrock (bare stone), soil
    /// (bare dirt) and grass, in that order.
    pub(crate) fn surfaces(&self) -> [MaterialId; 3] {
        [self.layers.bedrock, self.layers.soil, self.grass]
    }

    /// The column at `(x, z)` from its inputs. The height is
    /// `floor(inputs.height)` (saturating), the soil depth
    /// `floor(inputs.soil_depth)` clamped to `0..=255`, and the grass depth
    /// `thinnest + hash(x, z) mod (thickest - thinnest + 1)`.
    #[inline]
    pub fn column(&self, x: i32, z: i32, inputs: &ColumnInputs) -> Column {
        let parts = self.parts(x, z, inputs);
        Column {
            height: parts.height,
            surface: self.surfaces()[usize::from(parts.surface)],
            surface_depth: parts.surface_depth,
            soil_depth: parts.soil_depth,
        }
    }

    /// The thickest surface cover any column has, in voxels.
    pub(crate) fn max_cover(&self) -> u8 {
        let most = u64::from(self.grass_thin) + self.grass_choices - 1;
        u8::try_from(most).unwrap_or(u8::MAX)
    }

    /// [`column`](Self::column) with the surface material as its index in
    /// [`surfaces`](Self::surfaces), as a footprint stores it.
    #[inline]
    pub(crate) fn parts(&self, x: i32, z: i32, inputs: &ColumnInputs) -> Parts {
        let height = inputs.height.floor() as i32;
        let soil_depth = inputs.soil_depth.floor().clamp(0.0, 255.0) as u8;
        if inputs.slope >= self.cliff_slope || inputs.height >= self.rock_line {
            return Parts {
                height,
                surface: Parts::BEDROCK,
                surface_depth: 0,
                soil_depth: 0,
            };
        }
        if inputs.height < self.sea_level
            || inputs.moisture < self.dry_moisture
            || inputs.slope >= self.grass_max_slope
        {
            return Parts {
                height,
                surface: Parts::SOIL,
                surface_depth: 0,
                soil_depth,
            };
        }
        let pick = hash_column(self.seed, x, z) % self.grass_choices;
        Parts {
            height,
            surface: Parts::GRASS,
            surface_depth: self.grass_thin + pick as u8,
            soil_depth,
        }
    }
}

/// A [`Column`] whose surface is an index into [`Surface::surfaces`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Parts {
    pub height: i32,
    pub surface: u8,
    pub surface_depth: u8,
    pub soil_depth: u8,
}

impl Parts {
    pub const BEDROCK: u8 = 0;
    pub const SOIL: u8 = 1;
    pub const GRASS: u8 = 2;
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYERS: Layers = Layers {
        soil: MaterialId(2),
        bedrock: MaterialId(1),
    };

    fn inputs(height: f32, slope: f32, moisture: f32) -> ColumnInputs {
        ColumnInputs {
            height,
            slope,
            moisture,
            soil_depth: 64.0,
        }
    }

    #[test]
    fn layers_go_cover_soil_bedrock_from_the_top() {
        let c = Column {
            height: 10,
            surface: MaterialId(3),
            surface_depth: 2,
            soil_depth: 5,
        };
        let got: Vec<u16> = (3..12).map(|y| c.material_at(y, &LAYERS).0).collect();
        assert_eq!(got, [1, 1, 2, 2, 2, 3, 3, 0, 0]);
        assert_eq!(c.bedrock_below(), 5);
        assert_eq!(c.material_at(i32::MIN, &LAYERS), MaterialId(1));
        assert_eq!(c.material_at(i32::MAX, &LAYERS), MaterialId::AIR);
    }

    #[test]
    fn rules_pick_stone_dirt_or_grass() {
        let s = Surface::new(1, &SurfaceParams::default());
        assert_eq!(
            s.column(0, 0, &inputs(640.0, 0.1, 0.5)).surface,
            MaterialId(3)
        );
        assert_eq!(
            s.column(0, 0, &inputs(640.0, 2.0, 0.5)).surface,
            MaterialId(1)
        );
        assert_eq!(
            s.column(0, 0, &inputs(640.0, 1.0, 0.5)).surface,
            MaterialId(2)
        );
        assert_eq!(
            s.column(0, 0, &inputs(640.0, 0.1, -0.9)).surface,
            MaterialId(2)
        );
        assert_eq!(
            s.column(0, 0, &inputs(-64.0, 0.1, 0.5)).surface,
            MaterialId(2)
        );
        assert_eq!(
            s.column(0, 0, &inputs(64.0 * 200.0, 0.1, 0.5)).surface,
            MaterialId(1)
        );
        for x in 0..100 {
            let c = s.column(x, 7, &inputs(640.5, 0.1, 0.5));
            assert_eq!(c.height, 640);
            assert!((2..=5).contains(&c.surface_depth));
        }
    }

    #[test]
    fn materials_are_checked_against_the_table() {
        let p = SurfaceParams::default();
        assert!(
            p.check_materials(&MaterialTable::default())
                .unwrap_err()
                .len()
                == 3
        );
        let mut bad = p.clone();
        bad.grass = MaterialId::AIR;
        bad.grass_depth_voxels = [3, 1];
        assert_eq!(bad.problems().len(), 2);
        assert!(p.problems().is_empty());
    }
}
