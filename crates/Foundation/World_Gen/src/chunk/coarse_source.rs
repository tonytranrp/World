//! A coarse chunk as a region source: distant terrain at one material per
//! cell (decision 0006, the coarse tier).
//!
//! [`CoarseSource`] reads [`CoarseColumns`] sampled every `4^k` voxels and
//! describes a chunk whose every aligned `4^k` cube (a *cell*) is one
//! material: the material of its column sample at the cell's middle height,
//! or air when that middle is above the sample's surface. Building it gives a
//! chunk of uniform cells, a few thousand nodes instead of millions, which is
//! what rays far beyond the full-detail radius need.

use world_core::{MaterialId, RegionSource, VoxelPos};

use super::columns::CoarseColumns;

/// Coarse columns read as uniform cells.
#[derive(Clone, Debug)]
pub struct CoarseSource {
    columns: CoarseColumns,
    /// Per sample: the lowest `y` whose cell is air in that column.
    air_from: Vec<i32>,
}

impl CoarseSource {
    pub fn new(columns: CoarseColumns) -> Self {
        let spacing = columns.spacing() as i32;
        let air_from = columns
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
        Self { columns, air_from }
    }

    pub fn columns(&self) -> &CoarseColumns {
        &self.columns
    }

    /// The material of the cell holding `pos` (inside the chunk's footprint).
    fn cell(&self, pos: VoxelPos) -> MaterialId {
        let s = self.columns.spacing() as i32;
        let min = self.columns.chunk().min_voxel();
        let (i, j) = ((pos.x - min.x) / s, (pos.z - min.z) / s);
        let cell_y = pos.y.div_euclid(s) * s;
        self.columns
            .material(i as u32, cell_y + s / 2, j as u32)
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
        // the cube's bottom.
        let first = (dx / i64::from(s), dz / i64::from(s));
        let cells = i64::from(size / s);
        let mut lowest_air = i32::MIN;
        for j in first.1..(first.1 + cells).min(n) {
            for i in first.0..(first.0 + cells).min(n) {
                lowest_air = lowest_air.max(self.air_from[(i + n * j) as usize]);
            }
        }
        (min.y >= lowest_air).then_some(MaterialId::AIR)
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
