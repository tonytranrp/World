//! Continental scale: land and sea, base elevation, mountains, hills, and the
//! moisture and soil fields that biomes and layers are chosen from.
//!
//! Every field here is smooth at the metre scale (its finest octave is at
//! least [`MIN_FINEST_WAVELENGTH_M`]), so it is evaluated only on the
//! [`MACRO_SPACING`] lattice of world columns and interpolated in between by
//! the regional terrain field. [`Continental::sample`] is a pure function of
//! the seed, the parameters and the column.

use serde::{Deserialize, Serialize};

use crate::fingerprint::Fingerprint;
use crate::noise::{Fractal, FractalParams, PiecewiseLinear, derive, smoothstep};
use world_core::consts::VOXELS_PER_METER;

/// Spacing, in voxels, of the lattice the continental fields are sampled on:
/// one metre.
pub const MACRO_SPACING: i32 = 64;

/// The finest octave a continental field may have, in metres: 16 lattice
/// spacings, so bilinear interpolation between samples is invisible.
pub const MIN_FINEST_WAVELENGTH_M: f64 = 16.0;

/// The continental parameters, from the terrain config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinentalParams {
    /// Continentalness: the field that decides land, coast and sea.
    pub continent: FractalParams,
    /// Domain warp of the continentalness field, so coasts meander.
    pub warp: FractalParams,
    /// How far the warp moves sample points, in metres.
    pub warp_strength_m: f32,
    /// Continentalness (`-1` to `1`) to base elevation in metres, as points
    /// of a piecewise-linear curve with increasing continentalness.
    pub elevation_curve: Vec<[f32; 2]>,
    /// Where mountains may rise: fBm mapped through a smoothstep.
    pub mountain_mask: FractalParams,
    /// The smoothstep edges applied to the mountain mask's fBm (`-1` to `1`).
    pub mountain_mask_range: [f32; 2],
    /// Ridged noise giving mountain crests.
    pub mountains: FractalParams,
    /// Height of a full mountain above the base elevation, in metres.
    pub mountain_height_m: f32,
    /// Rolling hills everywhere.
    pub hills: FractalParams,
    /// Half the peak-to-trough height of the hills, in metres.
    pub hill_height_m: f32,
    /// Moisture, `-1` (dry) to `1` (wet), used by the surface rules.
    pub moisture: FractalParams,
    /// Variation of the soil depth.
    pub soil: FractalParams,
    /// Thinnest and thickest soil (dirt over stone), in metres.
    pub soil_depth_m: [f32; 2],
}

impl Default for ContinentalParams {
    fn default() -> Self {
        Self {
            continent: FractalParams::new(4096.0, 4),
            warp: FractalParams::new(2048.0, 2),
            warp_strength_m: 600.0,
            elevation_curve: vec![
                [-1.0, -48.0],
                [-0.35, -10.0],
                [-0.1, 2.0],
                [0.25, 18.0],
                [0.6, 40.0],
                [1.0, 64.0],
            ],
            mountain_mask: FractalParams::new(3072.0, 2),
            mountain_mask_range: [0.0, 0.45],
            mountains: FractalParams::new(1024.0, 5),
            mountain_height_m: 220.0,
            hills: FractalParams::new(256.0, 4),
            hill_height_m: 9.0,
            moisture: FractalParams::new(1536.0, 3),
            soil: FractalParams::new(48.0, 2),
            soil_depth_m: [0.5, 1.75],
        }
    }
}

impl ContinentalParams {
    /// Every problem with these parameters.
    pub fn problems(&self) -> Vec<String> {
        let min = MIN_FINEST_WAVELENGTH_M;
        let mut out = Vec::new();
        out.extend(self.continent.problems("continental.continent", min));
        out.extend(self.warp.problems("continental.warp", min));
        out.extend(
            self.mountain_mask
                .problems("continental.mountain_mask", min),
        );
        out.extend(self.mountains.problems("continental.mountains", min));
        out.extend(self.hills.problems("continental.hills", min));
        out.extend(self.moisture.problems("continental.moisture", min));
        out.extend(self.soil.problems("continental.soil", min));
        for p in PiecewiseLinear::problems(&self.elevation_curve) {
            out.push(format!("continental.elevation_curve: {p}"));
        }
        for (name, v, max) in [
            ("warp_strength_m", self.warp_strength_m, 4000.0),
            ("mountain_height_m", self.mountain_height_m, 4000.0),
            ("hill_height_m", self.hill_height_m, 1000.0),
        ] {
            if !(v.is_finite() && (0.0..=max).contains(&v)) {
                out.push(format!("continental.{name} {v} must be 0 to {max}"));
            }
        }
        let [lo, hi] = self.mountain_mask_range;
        if !(lo.is_finite() && hi.is_finite() && lo < hi) {
            out.push(format!(
                "continental.mountain_mask_range {:?} must be two finite values, low first",
                self.mountain_mask_range
            ));
        }
        let [thin, thick] = self.soil_depth_m;
        if !(thin.is_finite() && thick.is_finite() && 0.0 <= thin && thin <= thick && thick <= 3.9)
        {
            out.push(format!(
                "continental.soil_depth_m {:?} must be 0 <= thinnest <= thickest <= 3.9",
                self.soil_depth_m
            ));
        }
        out
    }

    pub(crate) fn fingerprint(&self, f: &mut Fingerprint) {
        for p in [
            &self.continent,
            &self.warp,
            &self.mountain_mask,
            &self.mountains,
            &self.hills,
            &self.moisture,
            &self.soil,
        ] {
            p.fingerprint(f);
        }
        f.u32(self.elevation_curve.len() as u32);
        for [x, y] in &self.elevation_curve {
            f.f32(*x).f32(*y);
        }
        f.f32(self.warp_strength_m)
            .f32(self.mountain_mask_range[0])
            .f32(self.mountain_mask_range[1])
            .f32(self.mountain_height_m)
            .f32(self.hill_height_m)
            .f32(self.soil_depth_m[0])
            .f32(self.soil_depth_m[1]);
    }

    /// The lowest and highest elevation the continental fields can produce,
    /// in metres, computed in `f64` from the curve and amplitudes.
    pub fn elevation_range_m(&self) -> (f64, f64) {
        let (lo, hi) = self
            .elevation_curve
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(f64::from(p[1])), hi.max(f64::from(p[1])))
            });
        let hills = f64::from(self.hill_height_m);
        (lo - hills, hi + f64::from(self.mountain_height_m) + hills)
    }
}

/// The continental fields at one lattice column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MacroSample {
    /// Elevation in voxels above `y = 0` (sea level is a surface parameter).
    pub height: f32,
    /// Moisture, `-1` to `1`.
    pub moisture: f32,
    /// Soil depth in voxels.
    pub soil_depth: f32,
}

/// The continental fields of one seed, ready to sample.
#[derive(Clone)]
pub struct Continental {
    continent: Fractal,
    warp_x: Fractal,
    warp_z: Fractal,
    warp_strength: f32,
    curve: PiecewiseLinear,
    mountain_mask: Fractal,
    mask_range: [f32; 2],
    mountains: Fractal,
    mountain_height: f32,
    hills: Fractal,
    hill_height: f32,
    moisture: Fractal,
    soil: Fractal,
    soil_thin: f32,
    soil_span: f32,
}

impl Continental {
    /// The fields of `seed` with validated parameters. Lengths in metres are
    /// converted to voxels by multiplying by 64, which is exact.
    pub fn new(seed: u64, params: &ContinentalParams) -> Result<Self, Vec<String>> {
        let vpm = VOXELS_PER_METER as f32;
        let points: Vec<[f32; 2]> = params
            .elevation_curve
            .iter()
            .map(|&[c, m]| [c, m * vpm])
            .collect();
        Ok(Self {
            continent: Fractal::new(derive(seed, "continental.continent"), &params.continent),
            warp_x: Fractal::new(derive(seed, "continental.warp_x"), &params.warp),
            warp_z: Fractal::new(derive(seed, "continental.warp_z"), &params.warp),
            warp_strength: params.warp_strength_m * vpm,
            curve: PiecewiseLinear::new(&points)?,
            mountain_mask: Fractal::new(
                derive(seed, "continental.mountain_mask"),
                &params.mountain_mask,
            ),
            mask_range: params.mountain_mask_range,
            mountains: Fractal::new(derive(seed, "continental.mountains"), &params.mountains),
            mountain_height: params.mountain_height_m * vpm,
            hills: Fractal::new(derive(seed, "continental.hills"), &params.hills),
            hill_height: params.hill_height_m * vpm,
            moisture: Fractal::new(derive(seed, "continental.moisture"), &params.moisture),
            soil: Fractal::new(derive(seed, "continental.soil"), &params.soil),
            soil_thin: params.soil_depth_m[0] * vpm,
            soil_span: (params.soil_depth_m[1] - params.soil_depth_m[0]) * vpm,
        })
    }

    /// The fields at column `(x, z)`, in this order:
    ///
    /// 1. warp: `wx = x + s * warp_x(x, z)`, `wz = z + s * warp_z(x, z)`;
    /// 2. base: `curve(continent(wx, wz))`;
    /// 3. mountains: `height * smoothstep(mask(x, z)) * r * r` with
    ///    `r = ridged(x, z)`;
    /// 4. hills: `height * hills(x, z)`;
    /// 5. elevation `(base + mountains) + hills`; moisture `moisture(x, z)`;
    ///    soil `thin + span * (0.5 + 0.5 * soil(x, z))`.
    pub fn sample(&self, x: i32, z: i32) -> MacroSample {
        let (xf, zf) = (x as f32, z as f32);
        let wx = xf + self.warp_strength * self.warp_x.fbm(xf, zf);
        let wz = zf + self.warp_strength * self.warp_z.fbm(xf, zf);
        let base = self.curve.eval(self.continent.fbm(wx, wz));
        let [lo, hi] = self.mask_range;
        let mask = smoothstep(lo, hi, self.mountain_mask.fbm(xf, zf));
        let r = self.mountains.ridged(xf, zf);
        let mountains = self.mountain_height * mask * (r * r);
        let hills = self.hill_height * self.hills.fbm(xf, zf);
        MacroSample {
            height: (base + mountains) + hills,
            moisture: self.moisture.fbm(xf, zf),
            soil_depth: self.soil_thin + self.soil_span * (0.5 + 0.5 * self.soil.fbm(xf, zf)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_parameters_are_valid() {
        assert_eq!(
            ContinentalParams::default().problems(),
            Vec::<String>::new()
        );
    }

    #[test]
    fn samples_stay_inside_the_elevation_range() {
        let params = ContinentalParams::default();
        let c = Continental::new(11, &params).unwrap();
        let (lo, hi) = params.elevation_range_m();
        let vpm = f64::from(VOXELS_PER_METER);
        for i in -50..50 {
            let s = c.sample(i * 7919, -i * 104_729 % 500_000);
            let h = f64::from(s.height);
            assert!(lo * vpm - 1.0 <= h && h <= hi * vpm + 1.0, "{h}");
            assert!((-1.0..=1.0).contains(&s.moisture));
            assert!(s.soil_depth >= 0.5 * 64.0 - 0.01 && s.soil_depth <= 1.75 * 64.0 + 0.01);
        }
    }

    #[test]
    fn bad_parameters_are_all_reported() {
        let mut p = ContinentalParams {
            mountain_mask_range: [1.0, 0.0],
            soil_depth_m: [2.0, 1.0],
            ..Default::default()
        };
        p.hills.wavelength_m = 1.0;
        p.elevation_curve.truncate(1);
        assert_eq!(p.problems().len(), 4, "{:?}", p.problems());
    }
}
