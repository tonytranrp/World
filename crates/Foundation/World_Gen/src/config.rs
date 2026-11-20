//! The terrain parameters: one RON config, `config/terrain.ron`.
//!
//! The defaults are the tuned values, and they must pass validation; the file
//! itself is added and registered by the layer that loads it (rule 10: tune
//! with data). Changing any value changes the generated world, so a save
//! records the generator id, which includes a fingerprint of these values.

use serde::{Deserialize, Serialize};

use crate::GeneratorVersion;
use crate::continental::ContinentalParams;
use crate::fingerprint::Fingerprint;
use crate::regional::RegionalParams;
use crate::regional::caves::CaveParams;
use crate::surface::SurfaceParams;
use world_core::consts::{VOXELS_PER_METER, WORLD_END_VOXEL};
use world_core::{ConfigAsset, MaterialTable};

/// Every parameter of terrain generation, grouped by stage.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainConfig {
    pub continental: ContinentalParams,
    pub regional: RegionalParams,
    pub surface: SurfaceParams,
    /// Caves, used from generator version 2. Missing from older files means
    /// the defaults.
    #[serde(default)]
    pub caves: CaveParams,
}

/// The lowest and highest column height a config can produce, in voxels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeightBounds {
    /// No column is lower.
    pub min: i32,
    /// No column is higher.
    pub max: i32,
}

/// The farthest the terrain may reach from `y = 0`, in metres, so that every
/// surface and the soil under it stay well inside the world.
pub const MAX_ELEVATION_M: f64 = 4096.0;

impl TerrainConfig {
    /// A hash of every value version 1 reads (the heightfield), in a fixed
    /// order.
    pub fn fingerprint(&self) -> u64 {
        let mut f = Fingerprint::new();
        self.continental.fingerprint(&mut f);
        self.regional.fingerprint(&mut f);
        self.surface.fingerprint(&mut f);
        f.finish()
    }

    /// A hash of every value `version` reads, part of [`crate::GeneratorId`]:
    /// version 1 ignores the caves, so they do not change its identity.
    pub fn fingerprint_for(&self, version: GeneratorVersion) -> u64 {
        if version < GeneratorVersion::V2 {
            return self.fingerprint();
        }
        let mut f = Fingerprint::new();
        f.u64(self.fingerprint());
        self.caves.fingerprint(&mut f);
        f.finish()
    }

    /// Conservative bounds on column heights: the elevation curve's range,
    /// plus or minus every amplitude, widened by two voxels for rounding.
    /// Computed in `f64` from the parameters, never from samples.
    pub fn height_bounds(&self) -> HeightBounds {
        let (lo_m, hi_m) = self.elevation_range_m();
        let vpm = f64::from(VOXELS_PER_METER);
        let clamp = |v: f64| v.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        HeightBounds {
            min: clamp((lo_m * vpm).floor() - 2.0),
            max: clamp((hi_m * vpm).ceil() + 2.0),
        }
    }

    fn elevation_range_m(&self) -> (f64, f64) {
        let (lo, hi) = self.continental.elevation_range_m();
        let relief = f64::from(self.regional.relief_height_m);
        (lo - relief, hi + relief)
    }

    /// Every material id the table does not define.
    pub fn check_materials(&self, table: &MaterialTable) -> Result<(), Vec<String>> {
        self.surface.check_materials(table)
    }

    /// Every problem in the config, as `ConfigAsset::validate` reports them.
    pub fn problems(&self) -> Vec<String> {
        let mut out = self.continental.problems();
        out.extend(self.regional.problems());
        out.extend(self.surface.problems());
        out.extend(self.caves.problems());
        if out.is_empty() {
            let (lo, hi) = self.elevation_range_m();
            if lo < -MAX_ELEVATION_M || hi > MAX_ELEVATION_M {
                out.push(format!(
                    "terrain spans {lo:.0} m to {hi:.0} m; it must stay within +-{MAX_ELEVATION_M} m \
                     (the world reaches {} m)",
                    f64::from(WORLD_END_VOXEL) / f64::from(VOXELS_PER_METER)
                ));
            }
        }
        out
    }
}

impl ConfigAsset for TerrainConfig {
    const NAME: &'static str = "terrain";
    const PATH: &'static str = "config/terrain.ron";

    fn validate(&self) -> Result<(), Vec<String>> {
        let problems = self.problems();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}
