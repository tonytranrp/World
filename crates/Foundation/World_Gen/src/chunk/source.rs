//! A chunk footprint as a region source for bottom-up tree building.
//!
//! [`ChunkSource`] answers the two questions a bottom-up builder asks
//! (decision 0006, "a bottom-up builder from column heights"): is this cube
//! all one material, and what are the 64 voxels of this leaf. Its method
//! signatures match World_Core's `RegionSource` trait, which is implemented
//! for it where both crates meet.
//!
//! `uniform` is exact when it answers: a cube is reported as air when every
//! column of its footprint is no higher than its lowest voxel, and as bedrock
//! when its highest voxel is below every column's [`Column::bedrock_below`].
//! Anything else (a cube cutting the surface, or lying entirely in one soil
//! layer) is `None`, which the builder treats as mixed and refines. The
//! per-footprint maximum height and minimum bedrock depth come from a 4x4
//! min/max pyramid over the chunk's columns, so cubes aligned to the tree
//! (side `4^k`, `k <= 5`, at a multiple of their side) cost one lookup.
//! Other cubes are answered by scanning their columns.

use super::columns::ChunkColumns;
use super::coord::CHUNK_SIZE;
use crate::surface::Column;
use world_core::{MaterialId, VoxelPos};

/// Footprint summary: the highest column and the lowest bedrock boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    max_height: i32,
    min_bedrock: i32,
}

impl Span {
    const EMPTY: Self = Self {
        max_height: i32::MIN,
        min_bedrock: i32::MAX,
    };

    fn of(column: &Column) -> Self {
        Self {
            max_height: column.height,
            min_bedrock: column.bedrock_below(),
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            max_height: self.max_height.max(other.max_height),
            min_bedrock: self.min_bedrock.min(other.min_bedrock),
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
        } else {
            None
        }
    }

    /// The 64 voxels of the leaf at `min`, in slot order `x + 4y + 16z`.
    /// Voxels whose column is outside the footprint are air.
    pub fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        for (slot, voxel) in voxels.iter_mut().enumerate() {
            let (x, y, z) = (slot as i32 & 3, (slot as i32 >> 2) & 3, slot as i32 >> 4);
            let pos = VoxelPos::new(
                min.x.wrapping_add(x),
                min.y.wrapping_add(y),
                min.z.wrapping_add(z),
            );
            *voxel = self.columns.material_at(pos).unwrap_or(MaterialId::AIR);
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
