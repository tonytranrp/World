//! A chunk footprint as a region source for bottom-up tree building.
//!
//! [`ChunkSource`] answers the two questions a bottom-up builder asks
//! (decision 0006, "a bottom-up builder from column heights"): is this cube
//! all one material, and what are the 64 voxels of this leaf. It implements
//! World_Core's [`RegionSource`], so `VoxelTree::build_region` builds a
//! generated chunk straight into the tree.
//!
//! `uniform` is exact when it answers: a cube is reported as air when every
//! column of its footprint is no higher than its lowest voxel, as bedrock
//! when its highest voxel is below every column's [`Column::bedrock_below`],
//! and as soil when it lies below every column's surface cover and above
//! every column's bedrock. Anything else (a cube cutting the surface or a
//! layer boundary) is `None`, which the builder treats as mixed and refines. The
//! per-footprint maximum height and minimum bedrock depth come from a 4x4
//! min/max pyramid over the chunk's columns, so cubes aligned to the tree
//! (side `4^k`, `k <= 5`, at a multiple of their side) cost one lookup.
//! Other cubes are answered by scanning their columns.

use super::columns::ChunkColumns;
use super::coord::CHUNK_SIZE;
use crate::surface::Column;
use world_core::{MaterialId, RegionSource, VoxelPos};

/// Footprint summary: the highest column and the lowest bedrock boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    max_height: i32,
    min_bedrock: i32,
    /// The lowest first voxel above a column's soil, over the footprint: the
    /// bottom of its surface cover, or its top when it has none.
    min_soil_end: i32,
    /// The highest lowest soil voxel over the footprint.
    max_soil_start: i32,
}

impl Span {
    const EMPTY: Self = Self {
        max_height: i32::MIN,
        min_bedrock: i32::MAX,
        min_soil_end: i32::MAX,
        max_soil_start: i32::MIN,
    };

    fn of(column: &Column) -> Self {
        let height = i64::from(column.height);
        let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        // Soil is depth surface_depth..soil_depth below the top voxel; with
        // no soil (soil_depth <= surface_depth) the range is empty.
        let soil_end = clamp(height - i64::from(column.surface_depth));
        let soil_start = clamp(height - i64::from(column.soil_depth.max(column.surface_depth)));
        Self {
            max_height: column.height,
            min_bedrock: column.bedrock_below(),
            min_soil_end: soil_end,
            max_soil_start: soil_start,
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            max_height: self.max_height.max(other.max_height),
            min_bedrock: self.min_bedrock.min(other.min_bedrock),
            min_soil_end: self.min_soil_end.min(other.min_soil_end),
            max_soil_start: self.max_soil_start.max(other.max_soil_start),
        }
    }
}

/// A chunk's columns plus the pyramid that makes [`uniform`](Self::uniform)
/// cheap. Columns extend through every `y`, so the source answers for any
/// cube whose footprint lies inside the chunk's.
#[derive(Clone, Debug)]
pub struct ChunkSource {
    columns: ChunkColumns,
    /// `pyramid[k - 1]` summarizes the aligned `4^k x 4^k` footprints, `k` in
    /// `1..=5`, `x` fastest.
    pyramid: Vec<Vec<Span>>,
}

impl ChunkSource {
    /// Builds the pyramid (about 1.4 M merges for a chunk).
    pub fn new(columns: ChunkColumns) -> Self {
        let mut pyramid: Vec<Vec<Span>> = Vec::new();
        let mut below: Vec<Span> = columns.columns().iter().map(Span::of).collect();
        let mut n = CHUNK_SIZE as usize;
        while n > 1 {
            let m = n / 4;
            let mut level = vec![Span::EMPTY; m * m];
            for z in 0..n {
                for x in 0..n {
                    let at = (z / 4) * m + x / 4;
                    level[at] = level[at].merge(below[z * n + x]);
                }
            }
            pyramid.push(level.clone());
            below = level;
            n = m;
        }
        Self { columns, pyramid }
    }

    pub fn columns(&self) -> &ChunkColumns {
        &self.columns
    }

    pub fn into_columns(self) -> ChunkColumns {
        self.columns
    }

    /// The material of every voxel of the cube at `min` with side `size`, if
    /// they are all one (air or bedrock, see the module docs); `None` when
    /// mixed, not provably uniform, empty, or not inside the footprint.
    pub fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
        let span = self.footprint(min, size)?;
        let bottom = i64::from(min.y);
        let top = bottom + i64::from(size) - 1;
        if bottom >= i64::from(span.max_height) {
            Some(MaterialId::AIR)
        } else if top < i64::from(span.min_bedrock) {
            Some(self.columns.layers().bedrock)
        } else if bottom >= i64::from(span.max_soil_start) && top < i64::from(span.min_soil_end) {
            Some(self.columns.layers().soil)
        } else {
            None
        }
    }

    /// The 64 voxels of the leaf at `min`, in slot order `x + 4y + 16z`.
    /// Voxels whose column is outside the footprint are air.
    pub fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        let chunk_min = self.columns.chunk().min_voxel();
        let layers = self.columns.layers();
        // Each of the leaf's 16 columns is read once for its 4 voxels.
        for z in 0..4 {
            for x in 0..4 {
                let local_x = i64::from(min.x) + i64::from(x) - i64::from(chunk_min.x);
                let local_z = i64::from(min.z) + i64::from(z) - i64::from(chunk_min.z);
                let column = u32::try_from(local_x)
                    .ok()
                    .zip(u32::try_from(local_z).ok())
                    .and_then(|(cx, cz)| self.columns.column(cx, cz));
                for y in 0..4 {
                    let slot = (x + 4 * y + 16 * z) as usize;
                    voxels[slot] = match column {
                        Some(column) => column.material_at(min.y.wrapping_add(y), &layers),
                        None => MaterialId::AIR,
                    };
                }
            }
        }
    }

    /// The summary of the footprint `[min.x, min.x + size) x [min.z, ...)`,
    /// or `None` if it is empty or leaves the chunk.
    fn footprint(&self, min: VoxelPos, size: u32) -> Option<Span> {
        let chunk_min = self.columns.chunk().min_voxel();
        let dx = i64::from(min.x) - i64::from(chunk_min.x);
        let dz = i64::from(min.z) - i64::from(chunk_min.z);
        let n = i64::from(CHUNK_SIZE);
        let s = i64::from(size);
        if size == 0 || dx < 0 || dz < 0 || dx + s > n || dz + s > n {
            return None;
        }
        let (dx, dz, s) = (dx as usize, dz as usize, s as usize);
        // An aligned power-of-four square is one pyramid entry.
        if size.is_power_of_two()
            && size.trailing_zeros().is_multiple_of(2)
            && dx.is_multiple_of(s)
            && dz.is_multiple_of(s)
        {
            let k = size.trailing_zeros() as usize / 2;
            if k == 0 {
                return self.columns.column(dx as u32, dz as u32).map(Span::of);
            }
            let m = CHUNK_SIZE as usize >> (2 * k);
            return self
                .pyramid
                .get(k - 1)
                .map(|level| level[(dz / s) * m + dx / s]);
        }
        let mut span = Span::EMPTY;
        for z in dz..dz + s {
            for x in dx..dx + s {
                let column = self.columns.columns()[z * CHUNK_SIZE as usize + x];
                span = span.merge(Span::of(&column));
            }
        }
        Some(span)
    }
}

impl RegionSource for ChunkSource {
    fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
        ChunkSource::uniform(self, min, size)
    }

    fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        ChunkSource::leaf(self, min, voxels);
    }
}
