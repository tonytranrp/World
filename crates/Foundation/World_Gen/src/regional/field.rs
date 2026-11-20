//! The lattice grids behind the terrain field, and the per-column inputs the
//! surface rules read.
//!
//! Two nested lattices of world columns, both aligned to multiples of their
//! spacing in world coordinates (so to every chunk and coarse level):
//!
//! - the macro lattice ([`MACRO_SPACING`], 64 voxels): [`MacroSample`]s from
//!   the continental fields;
//! - the detail lattice ([`DETAIL_SPACING`], 4 voxels): the terrain height
//!   `Hl = macro height interpolated at the point + regional relief`, and the
//!   slope from central differences of `Hl` over its four neighbours.
//!
//! A column's height and slope are bilinear interpolations of the detail
//! lattice; its moisture and soil depth of the macro lattice. Lattice values
//! are pure functions of their index, so the grids are only a cache and any
//! rectangle that covers a column gives it the same bits.

use super::{DETAIL_SPACING, Regional};
use crate::continental::{Continental, MACRO_SPACING, MacroSample};
use crate::noise::bilerp;

/// A rectangle of world columns: `x0..x0 + nx` by `z0..z0 + nz`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnRect {
    pub x0: i32,
    pub z0: i32,
    pub nx: u32,
    pub nz: u32,
}

impl ColumnRect {
    /// The single column `(x, z)`.
    pub const fn single(x: i32, z: i32) -> Self {
        Self {
            x0: x,
            z0: z,
            nx: 1,
            nz: 1,
        }
    }

    /// The last column on each axis. Rectangles are never empty here.
    fn last(&self) -> (i32, i32) {
        (
            self.x0 + self.nx.max(1) as i32 - 1,
            self.z0 + self.nz.max(1) as i32 - 1,
        )
    }
}

/// What the surface rules need to know about one column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnInputs {
    /// Terrain height in voxels: voxels below `floor(height)` are solid.
    pub height: f32,
    /// Steepness as rise over run (1.0 is 45 degrees).
    pub slope: f32,
    /// Moisture, `-1` to `1`.
    pub moisture: f32,
    /// Soil depth in voxels.
    pub soil_depth: f32,
}

/// A rectangle of the macro lattice: `samples[(m - m0) * nj + (j - j0)]` is
/// the sample at column `(64 j, 64 m)`.
pub struct MacroGrid {
    j0: i32,
    m0: i32,
    nj: usize,
    samples: Vec<MacroSample>,
}

impl MacroGrid {
    /// Every macro sample that columns of `rect`, and the detail points their
    /// height and slope use, interpolate from.
    pub fn covering(continental: &Continental, rect: &ColumnRect) -> Self {
        let (i_lo, k_lo, i_hi, k_hi) = detail_range(rect);
        let lo = |i: i32| (i * DETAIL_SPACING).div_euclid(MACRO_SPACING);
        let hi = |i: i32| (i * DETAIL_SPACING).div_euclid(MACRO_SPACING) + 1;
        let (j0, m0) = (lo(i_lo), lo(k_lo));
        let (j1, m1) = (hi(i_hi), hi(k_hi));
        let nj = (j1 - j0 + 1) as usize;
        let nm = (m1 - m0 + 1) as usize;
        let mut samples = Vec::with_capacity(nj * nm);
        for m in m0..=m1 {
            for j in j0..=j1 {
                samples.push(continental.sample(j * MACRO_SPACING, m * MACRO_SPACING));
            }
        }
        Self {
            j0,
            m0,
            nj,
            samples,
        }
    }

    #[inline]
    fn at(&self, j: i32, m: i32) -> &MacroSample {
        &self.samples[(m - self.m0) as usize * self.nj + (j - self.j0) as usize]
    }

    /// The four samples around column `(x, z)` and its fractions in the cell.
    #[inline]
    fn cell(&self, x: i32, z: i32) -> ([&MacroSample; 4], f32, f32) {
        let (j, m) = (x.div_euclid(MACRO_SPACING), z.div_euclid(MACRO_SPACING));
        let scale = 1.0 / MACRO_SPACING as f32;
        let fx = x.rem_euclid(MACRO_SPACING) as f32 * scale;
        let fz = z.rem_euclid(MACRO_SPACING) as f32 * scale;
        let corners = [
            self.at(j, m),
            self.at(j + 1, m),
            self.at(j, m + 1),
            self.at(j + 1, m + 1),
        ];
        (corners, fx, fz)
    }

    /// The continental elevation at column `(x, z)`, in voxels.
    #[inline]
    pub fn height(&self, x: i32, z: i32) -> f32 {
        let ([a, b, c, d], fx, fz) = self.cell(x, z);
        bilerp(a.height, b.height, c.height, d.height, fx, fz)
    }

    /// Moisture and soil depth at column `(x, z)`.
    #[inline]
    pub fn moisture_and_soil(&self, x: i32, z: i32) -> (f32, f32) {
        let ([a, b, c, d], fx, fz) = self.cell(x, z);
        (
            bilerp(a.moisture, b.moisture, c.moisture, d.moisture, fx, fz),
            bilerp(
                a.soil_depth,
                b.soil_depth,
                c.soil_depth,
                d.soil_depth,
                fx,
                fz,
            ),
        )
    }
}

/// The detail lattice indices a rectangle needs: the cells around its columns
/// plus one more on each side for the slope's central differences. Returns
/// `(i_lo, k_lo, i_hi, k_hi)`, inclusive.
fn detail_range(rect: &ColumnRect) -> (i32, i32, i32, i32) {
    let (x1, z1) = rect.last();
    (
        rect.x0.div_euclid(DETAIL_SPACING) - 1,
        rect.z0.div_euclid(DETAIL_SPACING) - 1,
        x1.div_euclid(DETAIL_SPACING) + 2,
        z1.div_euclid(DETAIL_SPACING) + 2,
    )
}

/// A rectangle of the detail lattice: heights `Hl` over the whole range and
/// slopes over its interior (one index in from every side).
pub struct DetailGrid {
    i0: i32,
    k0: i32,
    ni: usize,
    heights: Vec<f32>,
    slopes: Vec<f32>,
}

impl DetailGrid {
    /// Every detail point the columns of `rect` interpolate from. `macros`
    /// must cover `rect` (see [`MacroGrid::covering`]).
    ///
    /// `Hl(i, k) = macros.height(4i, 4k) + relief(4i, 4k)`; the slope at an
    /// interior point is `max(|Hl(i+1,k) - Hl(i-1,k)|, |Hl(i,k+1) - Hl(i,k-1)|)
    /// * 0.125`, rise over the 8-voxel run.
    pub fn covering(regional: &Regional, macros: &MacroGrid, rect: &ColumnRect) -> Self {
        let (i0, k0, i1, k1) = detail_range(rect);
        let ni = (i1 - i0 + 1) as usize;
        let nk = (k1 - k0 + 1) as usize;
        let mut heights = Vec::with_capacity(ni * nk);
        for k in k0..=k1 {
            for i in i0..=i1 {
                let (x, z) = (i * DETAIL_SPACING, k * DETAIL_SPACING);
                heights.push(macros.height(x, z) + regional.relief(x, z));
            }
        }
        let mut slopes = vec![0.0f32; ni * nk];
        for k in 1..nk - 1 {
            for i in 1..ni - 1 {
                let h = |di: usize, dk: usize| heights[(k + dk - 1) * ni + (i + di - 1)];
                let along_x = (h(2, 1) - h(0, 1)).abs();
                let along_z = (h(1, 2) - h(1, 0)).abs();
                slopes[k * ni + i] = along_x.max(along_z) * 0.125;
            }
        }
        Self {
            i0,
            k0,
            ni,
            heights,
            slopes,
        }
    }

    /// Height and slope at column `(x, z)`, interpolated from the four detail
    /// points around it with fractions `(x mod 4) * 0.25`, `(z mod 4) * 0.25`.
    #[inline]
    pub fn height_and_slope(&self, x: i32, z: i32) -> (f32, f32) {
        let (i, k) = (x.div_euclid(DETAIL_SPACING), z.div_euclid(DETAIL_SPACING));
        let scale = 1.0 / DETAIL_SPACING as f32;
        let fx = x.rem_euclid(DETAIL_SPACING) as f32 * scale;
        let fz = z.rem_euclid(DETAIL_SPACING) as f32 * scale;
        let at = (k - self.k0) as usize * self.ni + (i - self.i0) as usize;
        let n = self.ni;
        let h = &self.heights;
        let s = &self.slopes;
        (
            bilerp(h[at], h[at + 1], h[at + n], h[at + n + 1], fx, fz),
            bilerp(s[at], s[at + 1], s[at + n], s[at + n + 1], fx, fz),
        )
    }
}

/// The inputs of column `(x, z)`, from grids that cover it.
#[inline]
pub fn column_inputs(macros: &MacroGrid, detail: &DetailGrid, x: i32, z: i32) -> ColumnInputs {
    let (height, slope) = detail.height_and_slope(x, z);
    let (moisture, soil_depth) = macros.moisture_and_soil(x, z);
    ColumnInputs {
        height,
        slope,
        moisture,
        soil_depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::continental::ContinentalParams;
    use crate::regional::RegionalParams;

    fn fields() -> (Continental, Regional) {
        (
            Continental::new(5, &ContinentalParams::default()).unwrap(),
            Regional::new(5, &RegionalParams::default()),
        )
    }

    fn inputs(c: &Continental, r: &Regional, rect: &ColumnRect, x: i32, z: i32) -> ColumnInputs {
        let macros = MacroGrid::covering(c, rect);
        let detail = DetailGrid::covering(r, &macros, rect);
        column_inputs(&macros, &detail, x, z)
    }

    #[test]
    fn a_column_is_the_same_from_any_covering_rectangle() {
        let (c, r) = fields();
        let big = ColumnRect {
            x0: -70,
            z0: 130,
            nx: 150,
            nz: 90,
        };
        for (x, z) in [(-70, 130), (-1, 131), (0, 200), (79, 219), (13, 150)] {
            let a = inputs(&c, &r, &big, x, z);
            let b = inputs(&c, &r, &ColumnRect::single(x, z), x, z);
            assert_eq!(a.height.to_bits(), b.height.to_bits(), "({x}, {z})");
            assert_eq!(a.slope.to_bits(), b.slope.to_bits());
            assert_eq!(a.moisture.to_bits(), b.moisture.to_bits());
            assert_eq!(a.soil_depth.to_bits(), b.soil_depth.to_bits());
        }
    }

    #[test]
    fn slopes_are_non_negative_and_finite() {
        let (c, r) = fields();
        let rect = ColumnRect {
            x0: 1000,
            z0: -1000,
            nx: 64,
            nz: 64,
        };
        let macros = MacroGrid::covering(&c, &rect);
        let detail = DetailGrid::covering(&r, &macros, &rect);
        for z in -1000..-936 {
            for x in 1000..1064 {
                let i = column_inputs(&macros, &detail, x, z);
                assert!(i.slope >= 0.0 && i.slope.is_finite() && i.height.is_finite());
            }
        }
    }
}
