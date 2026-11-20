//! Generated columns of one chunk, at full resolution or on a coarse lattice.

use super::coord::{CHUNK_SIZE, ChunkCoord};
use crate::GeneratorId;
use crate::surface::{Column, Layers};
use world_core::{MaterialId, VoxelPos};

/// Every column of a chunk's 1024 x 1024 footprint. Columns do not depend on
/// the chunk's `y`, so one value serves every chunk stacked at that `x`, `z`.
///
/// Column `(x, z)` (chunk-local, each `0..1024`) is at index `x + 1024 z`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkColumns {
    pub(crate) generator: GeneratorId,
    pub(crate) chunk: ChunkCoord,
    pub(crate) layers: Layers,
    pub(crate) columns: Vec<Column>,
    pub(crate) height_range: (i32, i32),
}

impl ChunkColumns {
    /// The generator, version and parameters that produced these columns.
    pub fn generator(&self) -> GeneratorId {
        self.generator
    }

    /// The chunk whose footprint this is (its `y` is the one requested).
    pub fn chunk(&self) -> ChunkCoord {
        self.chunk
    }

    /// The soil and bedrock materials under every column's cover.
    pub fn layers(&self) -> Layers {
        self.layers
    }

    /// All columns, `x` fastest.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Column `(x, z)`, chunk-local; `None` outside `0..1024`.
    #[inline]
    pub fn column(&self, x: u32, z: u32) -> Option<&Column> {
        if x < CHUNK_SIZE && z < CHUNK_SIZE {
            self.columns.get((x + CHUNK_SIZE * z) as usize)
        } else {
            None
        }
    }

    /// The material of the voxel at chunk-local `x`, `z` and world `y`: air at
    /// and above the column's height, then cover, soil and bedrock. `None`
    /// when `x` or `z` is outside `0..1024`. This is a few comparisons, so a
    /// builder can fill a chunk column by column without any noise.
    #[inline]
    pub fn material(&self, x: u32, y: i32, z: u32) -> Option<MaterialId> {
        self.column(x, z).map(|c| c.material_at(y, &self.layers))
    }

    /// The material at a world voxel, if its column is in this footprint
    /// (any `y`).
    pub fn material_at(&self, pos: VoxelPos) -> Option<MaterialId> {
        let min = self.chunk.min_voxel();
        let (dx, dz) = (
            i64::from(pos.x) - i64::from(min.x),
            i64::from(pos.z) - i64::from(min.z),
        );
        let size = i64::from(CHUNK_SIZE);
        if !(0..size).contains(&dx) || !(0..size).contains(&dz) {
            return None;
        }
        self.material(dx as u32, pos.y, dz as u32)
    }

    /// The lowest and highest column height in the footprint. Voxels at or
    /// above the highest are air.
    pub fn height_range(&self) -> (i32, i32) {
        self.height_range
    }

    /// Every voxel below this `y` is bedrock in every column.
    pub fn bedrock_below(&self) -> i32 {
        self.columns
            .iter()
            .map(Column::bedrock_below)
            .min()
            .unwrap_or(i32::MIN)
    }

    /// Whether these columns are the footprint of `chunk` (any `y`) for
    /// `generator`.
    pub fn serves(&self, generator: GeneratorId, chunk: ChunkCoord) -> bool {
        self.generator == generator && self.chunk.x() == chunk.x() && self.chunk.z() == chunk.z()
    }

    /// The same columns as the footprint of `chunk`, which must share their
    /// `x` and `z` (`None` otherwise). Columns do not depend on `y`, so the
    /// result equals the generator's columns for `chunk` without computing
    /// any noise; it copies them (8 bytes a column).
    pub fn for_chunk(&self, chunk: ChunkCoord) -> Option<Self> {
        (self.chunk.x() == chunk.x() && self.chunk.z() == chunk.z()).then(|| Self {
            chunk,
            ..self.clone()
        })
    }

    /// The footprint's bounds, for deciding whole chunks without the columns
    /// (one pass over them).
    pub fn bounds(&self) -> FootprintBounds {
        FootprintBounds {
            generator: self.generator,
            x: self.chunk.x(),
            z: self.chunk.z(),
            height_range: self.height_range,
            bedrock_below: self.bedrock_below(),
        }
    }
}

/// What a chunk footprint's columns bound, for every chunk stacked on it:
/// 24 bytes against the columns' 8 MiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FootprintBounds {
    pub generator: GeneratorId,
    /// The footprint's chunk `x` and `z`.
    pub x: u32,
    pub z: u32,
    /// [`ChunkColumns::height_range`]: voxels at or above the highest are
    /// air.
    pub height_range: (i32, i32),
    /// [`ChunkColumns::bedrock_below`].
    pub bedrock_below: i32,
}

impl FootprintBounds {
    /// Whether these are the bounds of `chunk`'s footprint (any `y`) for
    /// `generator`.
    pub fn serves(&self, generator: GeneratorId, chunk: ChunkCoord) -> bool {
        self.generator == generator && self.x == chunk.x() && self.z == chunk.z()
    }
}

/// The deepest coarse level: one sample per chunk.
pub const MAX_COARSE_LEVEL: u32 = 5;

/// Columns of a chunk's footprint sampled every `4^level` voxels, for coarse
/// detail far from the player (decision 0006's coarse tier and the coarse
/// levels of decision 0007).
///
/// Sample `(i, j)` is the column at chunk-local `(i * spacing, j * spacing)`,
/// exactly the column the full-resolution footprint has there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoarseColumns {
    pub(crate) generator: GeneratorId,
    pub(crate) chunk: ChunkCoord,
    pub(crate) level: u32,
    pub(crate) layers: Layers,
    pub(crate) columns: Vec<Column>,
}

impl CoarseColumns {
    pub fn generator(&self) -> GeneratorId {
        self.generator
    }

    pub fn chunk(&self) -> ChunkCoord {
        self.chunk
    }

    pub fn layers(&self) -> Layers {
        self.layers
    }

    /// The level `k`: samples are `4^k` voxels apart.
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Voxels between samples: `4^level`.
    pub fn spacing(&self) -> u32 {
        1 << (2 * self.level)
    }

    /// Samples along each axis: `1024 / spacing`.
    pub fn samples_per_axis(&self) -> u32 {
        CHUNK_SIZE >> (2 * self.level)
    }

    /// All samples, `i` fastest.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Sample `(i, j)`; `None` outside `0..samples_per_axis()`.
    pub fn column(&self, i: u32, j: u32) -> Option<&Column> {
        let n = self.samples_per_axis();
        if i < n && j < n {
            self.columns.get((i + n * j) as usize)
        } else {
            None
        }
    }

    /// The material at world `y` of sample `(i, j)`.
    pub fn material(&self, i: u32, y: i32, j: u32) -> Option<MaterialId> {
        self.column(i, j).map(|c| c.material_at(y, &self.layers))
    }
}
