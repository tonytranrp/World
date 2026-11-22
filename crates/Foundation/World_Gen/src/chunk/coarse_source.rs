//! A coarse chunk as a region source: distant terrain at one material per
//! cell (decision 0006, the coarse tier).
//!
//! [`CoarseSource`] reads [`CoarseColumns`] sampled every `4^k` voxels and
//! describes a chunk whose every aligned `4^k` cube (a *cell*) is one
//! material: the material of its column sample at the cell's middle height,
//! or air when that middle is above the sample's surface. Building it gives a
//! chunk of uniform cells, a few thousand nodes instead of millions, which is
//! what rays far beyond the full-detail radius need.
//!
//! A *buried* cell, one whose cell above and four side neighbours are all
//! solid, can never be seen, so it takes its column's top material instead
//! of its own (decision 0029). Below the surface cover, soil and rock then
//! read as one material, and the level-2 nodes there collapse into one
//! uniform child word instead of a word per cell. A side neighbour outside
//! the chunk's footprint counts as the nearest sample inside it.

use world_core::{MaterialId, RegionSource, VoxelPos};

use super::columns::CoarseColumns;

/// Coarse columns read as uniform cells.
#[derive(Clone, Debug)]
pub struct CoarseSource {
    columns: CoarseColumns,
    /// Per sample: the lowest `y` whose cell is air in that column.
    air_from: Vec<i32>,
    /// Per sample: cells below this `y` are buried.
    buried_below: Vec<i32>,
    /// Per sample: the material of its top solid cell, which buried cells
    /// take (air when the column has none).
    top: Vec<MaterialId>,
}

impl CoarseSource {
    pub fn new(columns: CoarseColumns) -> Self {
        let spacing = columns.spacing() as i32;
        let air_from: Vec<i32> = columns
            .columns()
            .iter()
            .map(|c| {
                // A cell [y, y + s) is solid when y + s/2 < height, so the
                // first air cell starts at the first multiple of s with
                // y >= height - s/2.
                let threshold = i64::from(c.height) - i64::from(spacing / 2);
                let s = i64::from(spacing);
                (threshold.div_euclid(s) * s + if threshold.rem_euclid(s) == 0 { 0 } else { s })
                    .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
            })
            .collect();
        let n = columns.samples_per_axis() as usize;
        let at = |i: usize, j: usize| air_from[i + n * j];
        let mut buried_below = Vec::with_capacity(air_from.len());
        let mut top = Vec::with_capacity(air_from.len());
        for j in 0..n {
            for i in 0..n {
                // Buried: the cell above is solid (y + s < air_from) and so
                // is each side neighbour (y < its air_from).
                let sides = at(i.saturating_sub(1), j)
                    .min(at((i + 1).min(n - 1), j))
                    .min(at(i, j.saturating_sub(1)))
                    .min(at(i, (j + 1).min(n - 1)));
                let own = at(i, j);
                buried_below.push(own.saturating_sub(spacing).min(sides));
                let top_cell = own.saturating_sub(spacing);
                top.push(
                    columns
                        .material(i as u32, top_cell.saturating_add(spacing / 2), j as u32)
                        .unwrap_or(MaterialId::AIR),
                );
            }
        }
        Self {
            columns,
            air_from,
            buried_below,
            top,
        }
    }

    pub fn columns(&self) -> &CoarseColumns {
        &self.columns
    }

    /// Whether the cell holding `y` in sample `(i, j)` is buried, and the
    /// material it then takes.
    pub fn buried(&self, i: u32, y: i32, j: u32) -> Option<MaterialId> {
        let n = self.columns.samples_per_axis();
        if i >= n || j >= n {
            return None;
        }
        let k = (i + n * j) as usize;
        let s = self.columns.spacing() as i32;
        (y.div_euclid(s) * s < self.buried_below[k]).then_some(self.top[k])
    }

    /// The material of the cell holding `pos` (inside the chunk's footprint).
    fn cell(&self, pos: VoxelPos) -> MaterialId {
        let s = self.columns.spacing() as i32;
        let min = self.columns.chunk().min_voxel();
        let (i, j) = (((pos.x - min.x) / s) as u32, ((pos.z - min.z) / s) as u32);
        if let Some(m) = self.buried(i, pos.y, j) {
            return m;
        }
        let cell_y = pos.y.div_euclid(s) * s;
        self.columns
            .material(i, cell_y + s / 2, j)
            .unwrap_or(MaterialId::AIR)
    }
}

impl RegionSource for CoarseSource {
    fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
        let s = self.columns.spacing();
        let chunk = self.columns.chunk().min_voxel();
        let n = self.columns.samples_per_axis() as i64;
        let (dx, dz) = (
            i64::from(min.x) - i64::from(chunk.x),
            i64::from(min.z) - i64::from(chunk.z),
        );
        if dx < 0 || dz < 0 || dx >= n * i64::from(s) || dz >= n * i64::from(s) {
            return None;
        }
        if size <= s {
            // Inside one cell, which is uniform by construction.
            return Some(self.cell(min));
        }
        // Several cells: all air when every column's air starts at or below
        // the cube's bottom; all one material when every cell is buried in
        // columns of the same top material.
        let first = (dx / i64::from(s), dz / i64::from(s));
        let cells = i64::from(size / s);
        let mut lowest_air = i32::MIN;
        let mut lowest_buried = i32::MAX;
        let top = self.top[(first.0 + n * first.1) as usize];
        let mut same_top = true;
        for j in first.1..(first.1 + cells).min(n) {
            for i in first.0..(first.0 + cells).min(n) {
                let k = (i + n * j) as usize;
                lowest_air = lowest_air.max(self.air_from[k]);
                lowest_buried = lowest_buried.min(self.buried_below[k]);
                same_top &= self.top[k] == top;
            }
        }
        if min.y >= lowest_air {
            return Some(MaterialId::AIR);
        }
        let top_y = i64::from(min.y) + i64::from(size);
        (same_top && top_y <= i64::from(lowest_buried)).then_some(top)
    }

    fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        for (slot, voxel) in (0..).zip(voxels.iter_mut()) {
            *voxel = self.cell(VoxelPos::new(
                min.x + slot % 4,
                min.y + slot / 4 % 4,
                min.z + slot / 16,
            ));
        }
    }
}
