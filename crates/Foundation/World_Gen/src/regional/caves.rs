//! Caves (generator version 2): a 3D field carved out of the heightfield.
//!
//! Three independent 3D fBm fields are evaluated on a coarse lattice every
//! [`COARSE_CELL`] voxels (one metre): `a` and `b` (a tunnel runs where both
//! cross zero) and `c` (caverns open where it is high). They are
//! trilinearly upsampled to the cave lattice every [`CAVE_CELL`] voxels (a
//! quarter metre), where the cave value is
//!
//! ```text
//! f = min(max(|a|, |b|) - tunnel_radius, cavern_threshold - c)
//! ```
//!
//! and `f` is trilinearly interpolated to every voxel. Upsampling the smooth
//! fields first and combining them on the finer lattice keeps tunnel walls
//! sharp at a sixteenth of the noise evaluations. A solid voxel becomes air
//! when its `f` is below zero, it lies inside the cave band
//! `[bottom_m, top_m)`, and it is at least `cover_m` below its column's
//! surface. A cave-lattice point whose index is a multiple of four sits on
//! a coarse point and takes its value exactly (`t = 0`); every other one
//! interpolates within the coarse cell `floor(index / 4)`, so both paths
//! below pick the same corners.
//!
//! Every lattice value is a pure function of its global lattice index, and
//! interpolation runs in a fixed order with plain `f32` (decision 0008), so a
//! voxel is carved identically whether it comes from a whole chunk's grid or
//! a single point query, and neighbouring chunks agree at their faces.
//!
//! **Interval skipping.** Interpolation never leaves the range of its eight
//! corners, so a chunk's [`CaveGrid`] keeps the minimum and maximum corner of
//! every cell and a pyramid over them. A cube whose cells have no negative
//! corner is not carved at all; one whose cells all stay below
//! `-`[`CARVED_MARGIN`] is carved entirely (the margin absorbs rounding,
//! which is a few ulps of values below 4). Only cubes in between reach the
//! per-voxel test, so solid rock and open caverns cost one lookup each.
//! A cube smaller than a cell lies in one cell, where the interpolated field
//! is multilinear and so takes its extremes at the cube's eight corner
//! voxels; those eight values decide it, with the same margin on both sides.
//! A leaf (4 voxels) whose corners fall inside the margin, as leaves along
//! a cave wall do, is decided from all 64 of its values instead, computed
//! exactly as the leaf itself computes them ([`block_field`]), so it is
//! never read voxel by voxel only to come out uniform.

use serde::{Deserialize, Serialize};

use crate::chunk::{CHUNK_SIZE, ChunkCoord};
use crate::config::MAX_ELEVATION_M;
use crate::fingerprint::Fingerprint;
use crate::noise::{Fractal3, FractalParams, derive};
use crate::surface::Column;
use world_core::VoxelPos;
use world_core::consts::VOXELS_PER_METER;

/// Lattice spacing of the cave field, in voxels: a quarter metre.
pub const CAVE_CELL_SHIFT: u32 = 4;
pub const CAVE_CELL: i32 = 1 << CAVE_CELL_SHIFT;

/// Spacing of the lattice the noise fields are evaluated on: four cave
/// cells, one metre.
pub const COARSE_SHIFT: u32 = 2;
pub const COARSE_CELL: i32 = CAVE_CELL << COARSE_SHIFT;

/// Cells per chunk axis.
const CELLS: usize = (CHUNK_SIZE >> CAVE_CELL_SHIFT) as usize;

/// A cube is proven fully carved only when every corner value of its cells
/// is at most this far below zero.
pub const CARVED_MARGIN: f32 = 1.0e-3;

/// The finest octave a cave field may have, in metres: eight coarse cells.
pub const MIN_FINEST_WAVELENGTH_M: f64 = 8.0;

/// The cave parameters, from the terrain config.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaveParams {
    /// Off keeps version 2 terrain a plain heightfield.
    pub enabled: bool,
    /// The two fields whose shared zero crossing is a tunnel.
    pub tunnels: FractalParams,
    /// How far from zero both tunnel fields may be inside a tunnel, `0..=1`.
    /// Larger is wider.
    pub tunnel_radius: f32,
    /// The field whose high values are caverns.
    pub caverns: FractalParams,
    /// Caverns open where the cavern field exceeds this, `-1..=2` (above 1
    /// turns them off).
    pub cavern_threshold: f32,
    /// Vertical frequency multiplier of every cave field, `0.25..=4`. Above
    /// 1 makes caves wider than they are tall.
    pub y_scale: f32,
    /// The band caves may occupy, in metres: `bottom_m <= y < top_m`.
    pub bottom_m: f32,
    pub top_m: f32,
    /// Rock left untouched under every column's surface, in metres; 0 lets
    /// caves open to the sky.
    pub cover_m: f32,
}

impl Default for CaveParams {
    fn default() -> Self {
        Self {
            enabled: true,
            tunnels: FractalParams::new(40.0, 2),
            tunnel_radius: 0.06,
            caverns: FractalParams::new(64.0, 2),
            cavern_threshold: 0.6,
            y_scale: 1.75,
            bottom_m: -256.0,
            top_m: 320.0,
            cover_m: 3.0,
        }
    }
}

impl CaveParams {
    /// Every problem with these parameters.
    pub fn problems(&self) -> Vec<String> {
        let mut out = self
            .tunnels
            .problems("caves.tunnels", MIN_FINEST_WAVELENGTH_M);
        out.extend(
            self.caverns
                .problems("caves.caverns", MIN_FINEST_WAVELENGTH_M),
        );
        let in_range = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        if !in_range(self.tunnel_radius, 0.0, 1.0) {
            out.push(format!(
                "caves.tunnel_radius {} must be 0 to 1",
                self.tunnel_radius
            ));
        }
        if !in_range(self.cavern_threshold, -1.0, 2.0) {
            out.push(format!(
                "caves.cavern_threshold {} must be -1 to 2",
                self.cavern_threshold
            ));
        }
        if !in_range(self.y_scale, 0.25, 4.0) {
            out.push(format!("caves.y_scale {} must be 0.25 to 4", self.y_scale));
        }
        let limit = MAX_ELEVATION_M as f32;
        if !(in_range(self.bottom_m, -limit, limit)
            && in_range(self.top_m, -limit, limit)
            && self.bottom_m < self.top_m)
        {
            out.push(format!(
                "caves.bottom_m {} must be below caves.top_m {}, both within +-{limit} m",
                self.bottom_m, self.top_m
            ));
        }
        if !in_range(self.cover_m, 0.0, 64.0) {
            out.push(format!("caves.cover_m {} must be 0 to 64", self.cover_m));
        }
        out
    }

    pub(crate) fn fingerprint(&self, f: &mut Fingerprint) {
        f.u32(u32::from(self.enabled));
        self.tunnels.fingerprint(f);
        f.f32(self.tunnel_radius);
        self.caverns.fingerprint(f);
        f.f32(self.cavern_threshold)
            .f32(self.y_scale)
            .f32(self.bottom_m)
            .f32(self.top_m)
            .f32(self.cover_m);
    }
}

/// What the cave field does to a cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubeCaves {
    /// No voxel of the cube is carved.
    None,
    /// Every solid voxel of the cube is carved: the cube is air.
    All,
    /// Not proven either way.
    Mixed,
}

/// The cave fields of one seed, with lengths in voxels.
#[derive(Clone)]
pub struct Caves {
    tunnel_a: Fractal3,
    tunnel_b: Fractal3,
    cavern: Fractal3,
    tunnel_radius: f32,
    cavern_threshold: f32,
    /// `band.0 <= y < band.1`, voxels.
    band: (i32, i32),
    /// Voxels of untouched rock under each column's top.
    cover: i32,
}

impl Caves {
    /// The fields for validated `params`, or `None` when caves are off.
    pub fn new(seed: u64, params: &CaveParams) -> Option<Self> {
        if !params.enabled {
            return None;
        }
        let vpm = VOXELS_PER_METER as f32;
        let ys = params.y_scale;
        Some(Self {
            tunnel_a: Fractal3::new(derive(seed, "caves.tunnel_a"), &params.tunnels, ys),
            tunnel_b: Fractal3::new(derive(seed, "caves.tunnel_b"), &params.tunnels, ys),
            cavern: Fractal3::new(derive(seed, "caves.cavern"), &params.caverns, ys),
            tunnel_radius: params.tunnel_radius,
            cavern_threshold: params.cavern_threshold,
            band: (
                (params.bottom_m * vpm).floor() as i32,
                (params.top_m * vpm).floor() as i32,
            ),
            cover: (params.cover_m * vpm).floor() as i32,
        })
    }

    /// The raw fields `(a, b, c)` at global coarse point `(i, j, k)`, voxel
    /// position `(i, j, k) * COARSE_CELL`.
    pub fn coarse_fields(&self, i: i32, j: i32, k: i32) -> [f32; 3] {
        let shift = CAVE_CELL_SHIFT + COARSE_SHIFT;
        let x = i.wrapping_shl(shift) as f32;
        let y = j.wrapping_shl(shift) as f32;
        let z = k.wrapping_shl(shift) as f32;
        [
            self.tunnel_a.fbm(x, y, z),
            self.tunnel_b.fbm(x, y, z),
            self.cavern.fbm(x, y, z),
        ]
    }

    /// The cave value at a cave-lattice point, from the fields at the eight
    /// corners of its coarse cell and its offset `(di, dj, dk)` in `0..4`
    /// cave cells within it.
    #[inline]
    fn upsample(&self, corners: &[[f32; 3]; 8], (di, dj, dk): (i32, i32, i32)) -> f32 {
        let scale = 1.0 / (1 << COARSE_SHIFT) as f32;
        let t = (di as f32 * scale, dj as f32 * scale, dk as f32 * scale);
        let field = |n: usize| trilerp(&std::array::from_fn(|c| corners[c][n]), t);
        let tunnel = field(0).abs().max(field(1).abs()) - self.tunnel_radius;
        let cavern = self.cavern_threshold - field(2);
        tunnel.min(cavern)
    }

    /// The cave value at global cave-lattice point `(i, j, k)`, voxel
    /// position `(i, j, k) * CAVE_CELL`, computed from scratch.
    pub fn lattice_value(&self, i: i32, j: i32, k: i32) -> f32 {
        let mask = (1 << COARSE_SHIFT) - 1;
        let (ci, cj, ck) = (i >> COARSE_SHIFT, j >> COARSE_SHIFT, k >> COARSE_SHIFT);
        let corners: [[f32; 3]; 8] = std::array::from_fn(|n| {
            let (dx, dy, dz) = corner(n);
            self.coarse_fields(ci + dx, cj + dy, ck + dz)
        });
        self.upsample(&corners, (i & mask, j & mask, k & mask))
    }

    /// Whether voxel height `y` of `column` may be carved: inside the band
    /// and under the cover. Says nothing about the field.
    #[inline]
    pub fn carvable(&self, y: i32, column: &Column) -> bool {
        y >= self.band.0
            && y < self.band.1
            && i64::from(y) < i64::from(column.height) - i64::from(self.cover)
    }

    /// The heights `lo..hi` at which `column` may be carved:
    /// [`carvable`](Self::carvable) is `lo <= y && y < hi`.
    #[inline]
    pub fn carvable_range(&self, column: &Column) -> (i64, i64) {
        let hi = i64::from(self.band.1).min(i64::from(column.height) - i64::from(self.cover));
        (i64::from(self.band.0), hi)
    }

    /// Whether the solid voxel at `pos` in `column` is carved, computed from
    /// its own eight lattice corners.
    pub fn carved(&self, pos: VoxelPos, column: &Column) -> bool {
        if !self.carvable(pos.y, column) {
            return false;
        }
        let (i, j, k) = (
            pos.x >> CAVE_CELL_SHIFT,
            pos.y >> CAVE_CELL_SHIFT,
            pos.z >> CAVE_CELL_SHIFT,
        );
        let corners: [f32; 8] = std::array::from_fn(|n| {
            let (dx, dy, dz) = corner(n);
            self.lattice_value(i + dx, j + dy, k + dz)
        });
        trilinear(&corners, pos) < 0.0
    }

    /// The highest voxel height (exclusive) any column can be carved at,
    /// given the footprint's highest column.
    fn carve_top(&self, max_height: i32) -> i64 {
        i64::from(self.band.1).min(i64::from(max_height) - i64::from(self.cover))
    }

    /// The grid of chunk `chunk`, whose highest column is `max_height`, or
    /// `None` when no voxel of the chunk can be carved.
    pub fn grid(&self, chunk: ChunkCoord, max_height: i32) -> Option<CaveGrid> {
        let min = chunk.min_voxel();
        let lo = i64::from(self.band.0).max(i64::from(min.y));
        let hi = self
            .carve_top(max_height)
            .min(i64::from(min.y) + i64::from(CHUNK_SIZE));
        if lo >= hi {
            return None;
        }
        // Cells holding a carvable voxel, relative to the chunk.
        let first = ((lo - i64::from(min.y)) >> CAVE_CELL_SHIFT) as usize;
        let last = ((hi - 1 - i64::from(min.y)) >> CAVE_CELL_SHIFT) as usize;
        let rows = last - first + 1;
        let (gi, gj, gk) = (
            min.x >> CAVE_CELL_SHIFT,
            (min.y >> CAVE_CELL_SHIFT) + first as i32,
            min.z >> CAVE_CELL_SHIFT,
        );
        // The coarse fields over every coarse cell the cave lattice touches.
        let (ci, cj, ck) = (gi >> COARSE_SHIFT, gj >> COARSE_SHIFT, gk >> COARSE_SHIFT);
        let span = |g: i32, count: usize| {
            (((g + count as i32) >> COARSE_SHIFT) - (g >> COARSE_SHIFT)) as usize + 2
        };
        let (cx, cy, cz) = (span(gi, CELLS), span(gj, rows), span(gk, CELLS));
        let mut coarse = Vec::with_capacity(cx * cy * cz);
        for k in 0..cz {
            for j in 0..cy {
                for i in 0..cx {
                    coarse.push(self.coarse_fields(ci + i as i32, cj + j as i32, ck + k as i32));
                }
            }
        }
        let n = CELLS + 1;
        let mask = (1 << COARSE_SHIFT) - 1;
        let mut lattice = Vec::with_capacity(n * (rows + 1) * n);
        for k in 0..n {
            let gz = gk + k as i32;
            for j in 0..=rows {
                let gy = gj + j as i32;
                for i in 0..n {
                    let gx = gi + i as i32;
                    let (oi, oj, ok) = (
                        ((gx >> COARSE_SHIFT) - ci) as usize,
                        ((gy >> COARSE_SHIFT) - cj) as usize,
                        ((gz >> COARSE_SHIFT) - ck) as usize,
                    );
                    let corners: [[f32; 3]; 8] = std::array::from_fn(|c| {
                        let (dx, dy, dz) = corner(c);
                        let at =
                            ((ok + dz as usize) * cy + oj + dy as usize) * cx + oi + dx as usize;
                        coarse[at]
                    });
                    lattice.push(self.upsample(&corners, (gx & mask, gy & mask, gz & mask)));
                }
            }
        }
        let mut grid = CaveGrid {
            origin: min,
            first,
            rows,
            lattice,
            levels: Vec::new(),
        };
        grid.build_pyramid();
        Some(grid)
    }

    /// What the field does to the cube at `min` with side `size`, whose
    /// footprint's columns lie between `min_height` and `max_height`, using
    /// `grid` (the grid of `chunk`, or `None` when that chunk has no caves).
    /// Cubes outside `chunk` are [`CubeCaves::Mixed`] unless the band or the
    /// cover rules carving out.
    pub fn cube(
        &self,
        chunk: ChunkCoord,
        grid: Option<&CaveGrid>,
        min: VoxelPos,
        size: u32,
        heights: (i32, i32),
    ) -> CubeCaves {
        self.classify(chunk, grid, min, size, heights, true)
    }

    /// [`cube`](Self::cube) from whole-cell bounds alone, never looking inside
    /// a cell: slower to settle small cubes, and simple enough to check the
    /// fast answers against.
    #[cfg(test)]
    pub(crate) fn cube_by_cells(
        &self,
        chunk: ChunkCoord,
        grid: Option<&CaveGrid>,
        min: VoxelPos,
        size: u32,
        heights: (i32, i32),
    ) -> CubeCaves {
        self.classify(chunk, grid, min, size, heights, false)
    }

    #[inline]
    fn classify(
        &self,
        chunk: ChunkCoord,
        grid: Option<&CaveGrid>,
        min: VoxelPos,
        size: u32,
        (min_height, max_height): (i32, i32),
        inside_cells: bool,
    ) -> CubeCaves {
        let bottom = i64::from(min.y);
        let top = bottom + i64::from(size); // exclusive
        if top <= i64::from(self.band.0) || bottom >= self.carve_top(max_height) {
            return CubeCaves::None;
        }
        let c = chunk.min_voxel();
        let inside = |v: i32, o: i32| {
            let d = i64::from(v) - i64::from(o);
            d >= 0 && d + i64::from(size) <= i64::from(CHUNK_SIZE)
        };
        if !(inside(min.x, c.x) && inside(min.y, c.y) && inside(min.z, c.z)) {
            return CubeCaves::Mixed;
        }
        let Some(grid) = grid else {
            return CubeCaves::None;
        };
        let one_cell = |v: i32| (v & (CAVE_CELL - 1)) + size as i32 <= CAVE_CELL;
        let hi = if inside_cells && one_cell(min.x) && one_cell(min.y) && one_cell(min.z) {
            // One cell: the eight corner voxels bound the cube.
            let Some(corners) = grid.corners(min) else {
                return CubeCaves::None;
            };
            let last = size as i32 - 1;
            let (lo, hi) = range(&block_field(&corners, min, [0, last]));
            if lo >= CARVED_MARGIN {
                return CubeCaves::None;
            }
            if size == 4 && hi > -CARVED_MARGIN {
                // A leaf near a cave wall: rounding could put an inner voxel
                // past its corners, so decide from all 64 values, computed
                // exactly as the leaf computes them.
                let (lo, hi) = range(&block_field(&corners, min, [0, 1, 2, 3]));
                if lo >= 0.0 {
                    return CubeCaves::None;
                }
                // All carved needs `hi < 0`; the margin test below asks for
                // more, so pass a value that meets it exactly when it holds.
                if hi < 0.0 { -CARVED_MARGIN } else { hi }
            } else {
                hi
            }
        } else {
            let (lo, hi) = grid.bounds(min, size);
            if lo >= 0.0 {
                return CubeCaves::None;
            }
            hi
        };
        let under_cover = top <= i64::from(min_height) - i64::from(self.cover);
        if hi <= -CARVED_MARGIN
            && bottom >= i64::from(self.band.0)
            && top <= i64::from(self.band.1)
            && under_cover
        {
            CubeCaves::All
        } else {
            CubeCaves::Mixed
        }
    }
}

impl std::fmt::Debug for Caves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Caves")
            .field("band", &self.band)
            .field("cover", &self.cover)
            .finish_non_exhaustive()
    }
}

/// The cave lattice of one chunk, restricted to the rows of cells that can
/// hold a carved voxel, with per-cell corner bounds and a pyramid over them.
#[derive(Clone, Debug)]
pub struct CaveGrid {
    origin: VoxelPos,
    /// The first cell row (relative to the chunk) and how many rows the
    /// lattice covers.
    first: usize,
    rows: usize,
    /// `(CELLS + 1) x (rows + 1) x (CELLS + 1)` values, `x` fastest, then
    /// `y`, then `z`.
    lattice: Vec<f32>,
    /// `levels[l]` holds `(min, max)` corner bounds over aligned blocks of
    /// `2^l` cells per axis, `x` fastest; rows outside the lattice are
    /// `(inf, inf)`: never carved.
    levels: Vec<Vec<(f32, f32)>>,
}

impl CaveGrid {
    fn at(&self, i: usize, j: usize, k: usize) -> f32 {
        let n = CELLS + 1;
        self.lattice[(k * (self.rows + 1) + j) * n + i]
    }

    fn build_pyramid(&mut self) {
        let mut level = vec![(f32::INFINITY, f32::INFINITY); CELLS * CELLS * CELLS];
        for k in 0..CELLS {
            for j in 0..self.rows {
                for i in 0..CELLS {
                    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                    for n in 0..8 {
                        let v = self.at(i + (n & 1), j + (n >> 1 & 1), k + (n >> 2));
                        lo = lo.min(v);
                        hi = hi.max(v);
                    }
                    level[(k * CELLS + self.first + j) * CELLS + i] = (lo, hi);
                }
            }
        }
        let mut side = CELLS;
        let mut below = level;
        while side > 1 {
            let half = side / 2;
            let mut up = vec![(f32::INFINITY, f32::NEG_INFINITY); half * half * half];
            for k in 0..side {
                for j in 0..side {
                    for i in 0..side {
                        let (lo, hi) = below[(k * side + j) * side + i];
                        let u = &mut up[((k / 2) * half + j / 2) * half + i / 2];
                        u.0 = u.0.min(lo);
                        u.1 = u.1.max(hi);
                    }
                }
            }
            self.levels.push(std::mem::replace(&mut below, up));
            side = half;
        }
        self.levels.push(below);
    }

    /// Corner bounds over every cell the cube at `min` with side `size`
    /// touches; the cube lies inside the chunk.
    pub fn bounds(&self, min: VoxelPos, size: u32) -> (f32, f32) {
        let rel = |v: i32, o: i32| (v - o) as usize;
        let (x, y, z) = (
            rel(min.x, self.origin.x),
            rel(min.y, self.origin.y),
            rel(min.z, self.origin.z),
        );
        let s = size as usize;
        let cell = CAVE_CELL as usize;
        // An aligned power-of-two cube of at least one cell is one entry.
        if s.is_power_of_two()
            && s >= cell
            && x.is_multiple_of(s)
            && y.is_multiple_of(s)
            && z.is_multiple_of(s)
        {
            let l = (s / cell).trailing_zeros() as usize;
            let side = CELLS >> l;
            let (i, j, k) = (x / s, y / s, z / s);
            return self.levels[l][(k * side + j) * side + i];
        }
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let cells = |v: usize| (v / cell)..=((v + s - 1) / cell);
        for k in cells(z) {
            for j in cells(y) {
                for i in cells(x) {
                    let (a, b) = self.levels[0][(k * CELLS + j) * CELLS + i];
                    lo = lo.min(a);
                    hi = hi.max(b);
                }
            }
        }
        (lo, hi)
    }

    /// The eight corners of the cell holding `pos` (inside the chunk), or
    /// `None` when that cell lies outside the lattice rows (never carved).
    pub fn corners(&self, pos: VoxelPos) -> Option<[f32; 8]> {
        let i = ((pos.x - self.origin.x) >> CAVE_CELL_SHIFT) as usize;
        let j = ((pos.y - self.origin.y) >> CAVE_CELL_SHIFT) as usize;
        let k = ((pos.z - self.origin.z) >> CAVE_CELL_SHIFT) as usize;
        let j = j.checked_sub(self.first).filter(|&j| j < self.rows)?;
        let row = CELLS + 1;
        let plane = (self.rows + 1) * row;
        let base = k * plane + j * row + i;
        let cell = &self.lattice[base..=base + plane + row + 1];
        Some(std::array::from_fn(|n| {
            cell[(n & 1) + (n >> 1 & 1) * row + (n >> 2) * plane]
        }))
    }

    /// The `(min, max)` corner bound of the cell holding `pos`.
    pub fn cell_bounds(&self, pos: VoxelPos) -> (f32, f32) {
        let c = |v: i32, o: i32| ((v - o) >> CAVE_CELL_SHIFT) as usize;
        let (i, j, k) = (
            c(pos.x, self.origin.x),
            c(pos.y, self.origin.y),
            c(pos.z, self.origin.z),
        );
        self.levels[0][(k * CELLS + j) * CELLS + i]
    }
}

/// The offset of cell corner `n`: `(n & 1, n >> 1 & 1, n >> 2)`.
#[inline]
const fn corner(n: usize) -> (i32, i32, i32) {
    ((n & 1) as i32, (n >> 1 & 1) as i32, (n >> 2) as i32)
}

/// The field at the voxels `min + (offsets[i], offsets[j], offsets[k])`,
/// all in the cave cell whose corners are `c`, as `[k][j][i]`. Each value
/// equals [`trilinear`] at its voxel bit for bit: the lerps along `x` are
/// shared by every `j` and `k`, and those along `y` by every `k`, but each
/// value goes through the same operations in the same order.
#[inline]
pub fn block_field<const N: usize>(
    c: &[f32; 8],
    min: VoxelPos,
    offsets: [i32; N],
) -> [[[f32; N]; N]; N] {
    let mask = CAVE_CELL - 1;
    let scale = 1.0 / CAVE_CELL as f32;
    let t = |v: i32, d: i32| (v.wrapping_add(d) & mask) as f32 * scale;
    let lerp = |t: f32, a: f32, b: f32| a + t * (b - a);
    let xs: [[f32; 4]; N] = std::array::from_fn(|i| {
        let tx = t(min.x, offsets[i]);
        [
            lerp(tx, c[0], c[1]),
            lerp(tx, c[2], c[3]),
            lerp(tx, c[4], c[5]),
            lerp(tx, c[6], c[7]),
        ]
    });
    let ys: [[(f32, f32); N]; N] = std::array::from_fn(|j| {
        let ty = t(min.y, offsets[j]);
        std::array::from_fn(|i| {
            let [x00, x10, x01, x11] = xs[i];
            (lerp(ty, x00, x10), lerp(ty, x01, x11))
        })
    });
    std::array::from_fn(|k| {
        let tz = t(min.z, offsets[k]);
        std::array::from_fn(|j| std::array::from_fn(|i| lerp(tz, ys[j][i].0, ys[j][i].1)))
    })
}

/// The smallest and largest of a block of field values (which are never NaN).
#[inline]
fn range<const N: usize>(f: &[[[f32; N]; N]; N]) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in f.as_flattened().as_flattened() {
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }
    (lo, hi)
}

/// The field at `pos` from the eight corners of its cave cell: [`trilerp`]
/// with `t` the position within the cell, an exact multiple of
/// `1 / CAVE_CELL`.
#[inline]
pub fn trilinear(c: &[f32; 8], pos: VoxelPos) -> f32 {
    let mask = CAVE_CELL - 1;
    let scale = 1.0 / CAVE_CELL as f32;
    let t = (
        (pos.x & mask) as f32 * scale,
        (pos.y & mask) as f32 * scale,
        (pos.z & mask) as f32 * scale,
    );
    trilerp(c, t)
}

/// Trilinear interpolation of the eight corners of a cell (corner `n` at
/// offset [`corner`]`(n)`): lerps along `x`, then `y`, then `z`, each
/// `a + t(b - a)`.
#[inline]
pub fn trilerp(c: &[f32; 8], (tx, ty, tz): (f32, f32, f32)) -> f32 {
    let lerp = |t: f32, a: f32, b: f32| a + t * (b - a);
    let x00 = lerp(tx, c[0], c[1]);
    let x10 = lerp(tx, c[2], c[3]);
    let x01 = lerp(tx, c[4], c[5]);
    let x11 = lerp(tx, c[6], c[7]);
    let y0 = lerp(ty, x00, x10);
    let y1 = lerp(ty, x01, x11);
    lerp(tz, y0, y1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caves() -> Caves {
        Caves::new(11, &CaveParams::default()).unwrap()
    }

    #[test]
    fn defaults_are_valid_and_bad_values_are_all_reported() {
        assert!(CaveParams::default().problems().is_empty());
        let bad = CaveParams {
            tunnel_radius: -1.0,
            cavern_threshold: f32::NAN,
            y_scale: 9.0,
            bottom_m: 10.0,
            top_m: 0.0,
            cover_m: -1.0,
            ..CaveParams::default()
        };
        assert_eq!(bad.problems().len(), 5, "{:?}", bad.problems());
    }

    #[test]
    fn interpolation_hits_the_corners_and_stays_between_them() {
        let c = [0.5, -0.25, 1.0, 0.0, -1.0, 0.75, 0.125, -0.5];
        assert_eq!(trilinear(&c, VoxelPos::new(0, 0, 0)), 0.5);
        assert_eq!(trilinear(&c, VoxelPos::new(32, -16, 48)), 0.5);
        let mut s = crate::noise::Stream::new(2);
        for _ in 0..10_000 {
            let mut v = || (s.next_u64() % 4096) as i32 - 2048;
            let p = VoxelPos::new(v(), v(), v());
            let f = trilinear(&c, p);
            assert!((-1.0..=1.0).contains(&f), "{f}");
        }
    }

    #[test]
    fn a_grid_agrees_with_point_queries_and_its_bounds_hold() {
        let caves = caves();
        // A chunk deep underground, inside the band.
        let chunk = ChunkCoord::new(512, 506, 512).unwrap();
        let grid = caves.grid(chunk, i32::MAX / 2).unwrap();
        let column = Column {
            height: i32::MAX / 2,
            ..Column::default()
        };
        let min = chunk.min_voxel();
        let mut carved = 0;
        let mut s = crate::noise::Stream::new(3);
        for _ in 0..20_000 {
            let mut v = || (s.next_u64() % u64::from(CHUNK_SIZE)) as i32;
            let p = VoxelPos::new(min.x + v(), min.y + v(), min.z + v());
            let corners = grid.corners(p).unwrap();
            let f = trilinear(&corners, p);
            assert_eq!(f < 0.0, caves.carved(p, &column), "at {p:?}");
            let (lo, hi) = grid.cell_bounds(p);
            assert!(lo <= f && f <= hi);
            carved += usize::from(f < 0.0);
        }
        assert!(carved > 0, "the default caves carve something");
        assert!(carved < 20_000 / 2, "and leave most rock standing");
    }
}
