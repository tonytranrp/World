//! Regional scale: the unified terrain field.
//!
//! Version 1 terrain is a heightfield. The field combines the continental
//! elevation (interpolated from its one-metre lattice) with regional relief
//! (metre-to-decimetre bumps) sampled on a finer [`DETAIL_SPACING`] lattice,
//! and derives the slope the surface rules need. Caves, overhangs and rivers
//! will join here as signed distance fields in a later generator version.
//!
//! [`field`] holds the lattice grids. Their values depend only on the lattice
//! index, never on which rectangle of columns asked for them, so a column is
//! identical whether it is generated in a whole chunk, a coarse pass or alone.

pub mod field;

use serde::{Deserialize, Serialize};

use crate::fingerprint::Fingerprint;
use crate::noise::{Fractal, FractalParams, derive};
use world_core::consts::VOXELS_PER_METER;

pub use field::{ColumnInputs, ColumnRect, DetailGrid, MacroGrid};

/// Spacing, in voxels, of the lattice the regional relief is sampled on.
pub const DETAIL_SPACING: i32 = 4;

/// The finest octave the relief may have, in metres: 16 voxels, four lattice
/// spacings.
pub const MIN_FINEST_WAVELENGTH_M: f64 = 0.25;

/// The regional parameters, from the terrain config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionalParams {
    /// Small-scale relief on top of the continental elevation.
    pub relief: FractalParams,
    /// Half the peak-to-trough height of the relief, in metres.
    pub relief_height_m: f32,
}

impl Default for RegionalParams {
    fn default() -> Self {
        Self {
            relief: FractalParams::new(16.0, 6),
            relief_height_m: 1.0,
        }
    }
}

impl RegionalParams {
    /// Every problem with these parameters.
    pub fn problems(&self) -> Vec<String> {
        let mut out = self
            .relief
            .problems("regional.relief", MIN_FINEST_WAVELENGTH_M);
        let h = self.relief_height_m;
        if !(h.is_finite() && (0.0..=100.0).contains(&h)) {
            out.push(format!("regional.relief_height_m {h} must be 0 to 100"));
        }
        out
    }

    pub(crate) fn fingerprint(&self, f: &mut Fingerprint) {
        self.relief.fingerprint(f);
        f.f32(self.relief_height_m);
    }
}

/// The regional fields of one seed, ready to sample.
#[derive(Clone)]
pub struct Regional {
    relief: Fractal,
    relief_height: f32,
}

impl Regional {
    pub fn new(seed: u64, params: &RegionalParams) -> Self {
        Self {
            relief: Fractal::new(derive(seed, "regional.relief"), &params.relief),
            relief_height: params.relief_height_m * VOXELS_PER_METER as f32,
        }
    }

    /// The relief at column `(x, z)` in voxels: `height * fbm(x, z)`.
    pub fn relief(&self, x: i32, z: i32) -> f32 {
        self.relief_height * self.relief.fbm(x as f32, z as f32)
    }
}
