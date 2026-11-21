//! Generated columns of one chunk, at full resolution or on a coarse lattice.

use std::sync::Arc;

use super::coord::{CHUNK_SIZE, ChunkCoord};
use crate::GeneratorId;
use crate::surface::{Column, Layers, Parts};
use world_core::{MaterialId, VoxelPos};

/// Every column of a chunk's 1024 x 1024 footprint. Columns do not depend on
/// the chunk's `y`, so one value serves every chunk stacked at that `x`, `z`.
///
/// Column `(x, z)` (chunk-local, each `0..1024`) is at index `x + 1024 z`.
///
/// The columns are stored packed, four bytes each (4 MiB a footprint), and
/// shared: cloning the value, or [`for_chunk`](Self::for_chunk), copies a
/// pointer, not the columns. A footprint that does not fit the packed form
/// (see [`Store`]) keeps its columns at eight bytes each; no shipped terrain
/// produces one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkColumns {
    pub(crate) generator: GeneratorId,
    pub(crate) chunk: ChunkCoord,
    pub(crate) layers: Layers,
    pub(crate) store: Store,
    pub(crate) height_range: (i32, i32),
}

/// How a footprint's columns are kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    /// Four bytes a column (see [`Packing`]).
    Packed(Packing, Arc<[u32]>),
    /// Eight bytes a column, when some column does not pack: its height is
    /// more than 32,767 voxels (512 m) from the first column's, or the
    /// configured grass can be thicker than 63 voxels.
    Wide(Arc<[Column]>),
}

/// What a packed column word is relative to. A word holds, from the lowest
/// bit, the height above `base` (16 bits), the soil depth (8 bits), the
/// index of the surface material in `surfaces` (2 bits) and the cover depth
/// (6 bits). The surface rules give every column one of three surface
/// materials, a soil depth of at most 255 voxels and a cover as thick as the
/// configured grass (2 to 5 voxels by default), so only the height needs a
/// base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Packing {
    base: i32,
    /// The ids of bedrock, soil and grass from the surface rules, 16 bits
    /// each from the lowest, so unpacking selects one with a shift.
    surfaces: u64,
}

const HEIGHT_BITS: u32 = 16;
const SOIL_SHIFT: u32 = 16;
const SURFACE_SHIFT: u32 = 24;
const COVER_SHIFT: u32 = 26;
const MAX_COVER: u8 = (1 << (32 - COVER_SHIFT)) - 1;

impl Packing {
    /// A packing for a footprint whose first column is `first`: heights
    /// within 32,768 voxels below and 32,767 above it pack.
    pub(crate) fn new(first: i32, surfaces: [MaterialId; 3]) -> Self {
        let half = 1i64 << (HEIGHT_BITS - 1);
        let base = (i64::from(first) - half).clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        Self {
            base: base as i32,
            surfaces: surfaces
                .iter()
                .rev()
                .fold(0, |bits, id| bits << 16 | u64::from(id.0)),
        }
    }

    /// The word of a column. It holds the column exactly when
    /// [`holds`](Self::holds) its height and cover depth; the generator
    /// checks that once per footprint, from its height range and the
    /// thickest cover the surface rules give, not per column.
    #[inline(always)]
    pub(crate) fn pack(&self, parts: &Parts) -> u32 {
        (parts.height.wrapping_sub(self.base) as u32 & ((1 << HEIGHT_BITS) - 1))
            | u32::from(parts.soil_depth) << SOIL_SHIFT
            | u32::from(parts.surface & 3) << SURFACE_SHIFT
            | u32::from(parts.surface_depth & MAX_COVER) << COVER_SHIFT
    }

    /// The lowest [`Column::bedrock_below`] of `words`, `i32::MIN` when
    /// there are none: the lowest height above the base less the deeper
    /// layer, in one pass over the words.
    fn bedrock_below(&self, words: &[u32]) -> i32 {
        let lowest = words
            .iter()
            .map(|&w| {
                let layer = (w >> SOIL_SHIFT & 0xFF).max(w >> COVER_SHIFT);
                (w & ((1 << HEIGHT_BITS) - 1)) as i32 - layer as i32
            })
            .min();
        lowest.map_or(i32::MIN, |d| {
            (i64::from(self.base) + i64::from(d)).clamp(i64::from(i32::MIN), i64::from(i32::MAX))
                as i32
        })
    }

    /// Whether every height in `lo..=hi` and every cover depth up to
    /// `max_cover` packs exactly.
    pub(crate) fn holds(&self, (lo, hi): (i32, i32), max_cover: u8) -> bool {
        let range = 0..1i64 << HEIGHT_BITS;
        range.contains(&(i64::from(lo) - i64::from(self.base)))
            && range.contains(&(i64::from(hi) - i64::from(self.base)))
            && max_cover <= MAX_COVER
    }

    #[inline(always)]
    pub(crate) fn unpack(&self, word: u32) -> Column {
        Column {
            height: self
                .base
                .wrapping_add((word & ((1 << HEIGHT_BITS) - 1)) as i32),
            surface: MaterialId((self.surfaces >> (word >> (SURFACE_SHIFT - 4) & 0x30)) as u16),
            surface_depth: (word >> COVER_SHIFT) as u8,
            soil_depth: (word >> SOIL_SHIFT) as u8,
        }
    }
}

/// How the builder reads a footprint's columns as stored: the layer
/// boundaries of a column measured from an origin, so a packed column needs
/// no base added (the reader subtracts the origin from the leaf's heights
/// once instead), and the whole column only where caves need it.
pub(crate) trait ColumnRead: Copy {
    type Stored: Copy;

    /// What the boundaries are measured from.
    fn origin(&self) -> i64;

    /// The column's top (its first air voxel), the first voxel of its
    /// cover and the first of its soil, from the origin; and its surface.
    fn boundaries(&self, stored: Self::Stored) -> (i64, i64, i64, MaterialId);

    fn column(&self, stored: Self::Stored) -> Column;
}

/// Reads eight-byte columns.
#[derive(Clone, Copy)]
pub(crate) struct WideRead;

impl ColumnRead for WideRead {
    type Stored = Column;

    #[inline(always)]
    fn origin(&self) -> i64 {
        0
    }

    #[inline(always)]
    fn boundaries(&self, c: Column) -> (i64, i64, i64, MaterialId) {
        let height = i64::from(c.height);
        (
            height,
            height - i64::from(c.surface_depth),
            height - i64::from(c.soil_depth),
            c.surface,
        )
    }

    #[inline(always)]
    fn column(&self, c: Column) -> Column {
        c
    }
}

impl ColumnRead for Packing {
    type Stored = u32;

    #[inline(always)]
    fn origin(&self) -> i64 {
        i64::from(self.base)
    }

    #[inline(always)]
    fn boundaries(&self, word: u32) -> (i64, i64, i64, MaterialId) {
        let height = i64::from(word & ((1 << HEIGHT_BITS) - 1));
        (
            height,
            height - i64::from(word >> COVER_SHIFT),
            height - i64::from(word >> SOIL_SHIFT & 0xFF),
            MaterialId((self.surfaces >> (word >> (SURFACE_SHIFT - 4) & 0x30)) as u16),
        )
    }

    #[inline(always)]
    fn column(&self, word: u32) -> Column {
        self.unpack(word)
    }
}

impl Store {
    /// `columns`, packed if every one packs (see [`Packing::new`]) and wide
    /// otherwise; `surfaces` are bedrock, soil and grass. Footprints are
    /// packed as they are generated (`Stages::columns`); this is their
    /// reference.
    #[cfg(test)]
    pub(crate) fn new(columns: &[Column], surfaces: [MaterialId; 3]) -> Self {
        let Some(first) = columns.first() else {
            return Store::Wide(Arc::new([]));
        };
        let packing = Packing::new(first.height, surfaces);
        let mut exact = true;
        let words: Arc<[u32]> = columns
            .iter()
            .map(|c| {
                let surface = surfaces.iter().position(|&m| m == c.surface);
                exact &= surface.is_some() && packing.holds((c.height, c.height), c.surface_depth);
                packing.pack(&Parts {
                    height: c.height,
                    surface: surface.unwrap_or(0) as u8,
                    surface_depth: c.surface_depth,
                    soil_depth: c.soil_depth,
                })
            })
            .collect();
        if exact {
            Store::Packed(packing, words)
        } else {
            Store::Wide(columns.into())
        }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            Store::Packed(_, words) => words.len(),
            Store::Wide(columns) => columns.len(),
        }
    }

    /// Column `i`; panics past the end.
    #[inline]
    fn get(&self, i: usize) -> Column {
        match self {
            Store::Packed(packing, words) => packing.unpack(words[i]),
            Store::Wide(columns) => columns[i],
        }
    }
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
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Column> + '_ {
        (0..self.store.len()).map(|i| self.store.get(i))
    }

    /// All columns in a vector, `x` fastest.
    pub fn to_vec(&self) -> Vec<Column> {
        self.iter().collect()
    }

    /// Column `(x, z)`, chunk-local; `None` outside `0..1024`.
    #[inline]
    pub fn column(&self, x: u32, z: u32) -> Option<Column> {
        if x < CHUNK_SIZE && z < CHUNK_SIZE {
            Some(self.get((x + CHUNK_SIZE * z) as usize))
        } else {
            None
        }
    }

    /// Column `i` (`x + 1024 z`); panics past the end.
    #[inline]
    pub(crate) fn get(&self, i: usize) -> Column {
        self.store.get(i)
    }

    /// Bytes the columns occupy on the heap (shared by every clone).
    pub fn heap_bytes(&self) -> usize {
        match &self.store {
            Store::Packed(_, words) => std::mem::size_of_val::<[u32]>(words),
            Store::Wide(columns) => std::mem::size_of_val::<[Column]>(columns),
        }
    }

    /// Whether the columns are packed at four bytes each.
    pub fn is_packed(&self) -> bool {
        matches!(self.store, Store::Packed(..))
    }

    /// Whether `other` points at the same column storage (a clone, or
    /// [`for_chunk`](Self::for_chunk) of this or of a clone).
    pub fn shares_columns(&self, other: &Self) -> bool {
        match (&self.store, &other.store) {
            (Store::Packed(_, a), Store::Packed(_, b)) => Arc::ptr_eq(a, b),
            (Store::Wide(a), Store::Wide(b)) => Arc::ptr_eq(a, b),
            _ => false,
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

    /// Every voxel below this `y` is bedrock in every column (one pass over
    /// the columns).
    pub fn bedrock_below(&self) -> i32 {
        match &self.store {
            Store::Packed(packing, words) => packing.bedrock_below(words),
            Store::Wide(columns) => columns
                .iter()
                .map(Column::bedrock_below)
                .min()
                .unwrap_or(i32::MIN),
        }
    }

    /// Whether these columns are the footprint of `chunk` (any `y`) for
    /// `generator`.
    pub fn serves(&self, generator: GeneratorId, chunk: ChunkCoord) -> bool {
        self.generator == generator && self.chunk.x() == chunk.x() && self.chunk.z() == chunk.z()
    }

    /// The same columns as the footprint of `chunk`, which must share their
    /// `x` and `z` (`None` otherwise). Columns do not depend on `y`, so the
    /// result equals the generator's columns for `chunk` without computing
    /// any noise; it shares them rather than copying.
    pub fn for_chunk(&self, chunk: ChunkCoord) -> Option<Self> {
        (self.chunk.x() == chunk.x() && self.chunk.z() == chunk.z()).then(|| Self {
            chunk,
            ..self.clone()
        })
    }

    /// The footprint's bounds, for deciding whole chunks without the columns.
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
/// 24 bytes against the columns' 4 MiB.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChunkCoord, Generator, GeneratorVersion, TerrainConfig};
    use proptest::prelude::*;

    const SURFACES: [MaterialId; 3] = [MaterialId(1), MaterialId(2), MaterialId(3)];

    fn unpacked(store: &Store) -> Vec<Column> {
        (0..store.len()).map(|i| store.get(i)).collect()
    }

    fn column() -> impl Strategy<Value = Column> {
        (-40_000i32..40_000, 0usize..3, 0u8..=63, any::<u8>()).prop_map(
            |(height, surface, surface_depth, soil_depth)| Column {
                height,
                surface: SURFACES[surface],
                surface_depth,
                soil_depth,
            },
        )
    }

    proptest! {
        /// Whatever the columns, they come back unchanged, packed when every
        /// height is within the 16 bits around the first.
        #[test]
        fn columns_round_trip_packed_or_wide(columns in prop::collection::vec(column(), 1..200)) {
            let store = Store::new(&columns, SURFACES);
            prop_assert_eq!(unpacked(&store), columns.clone());
            let first = i64::from(columns[0].height);
            let fits = columns
                .iter()
                .all(|c| (-32_768..=32_767).contains(&(i64::from(c.height) - first)));
            prop_assert_eq!(matches!(store, Store::Packed(..)), fits);
            if let Store::Packed(packing, words) = &store {
                let want = columns.iter().map(Column::bedrock_below).min().unwrap();
                prop_assert_eq!(packing.bedrock_below(words), want);
            }
        }
    }

    #[test]
    fn columns_that_do_not_pack_keep_every_column_wide() {
        let plain = Column {
            height: 100,
            surface: SURFACES[2],
            surface_depth: 3,
            soil_depth: 90,
        };
        for odd in [
            Column {
                height: 100 + 32_768,
                ..plain
            },
            Column {
                height: 100 - 32_769,
                ..plain
            },
            Column {
                surface_depth: 64,
                ..plain
            },
            Column {
                surface: MaterialId(9),
                ..plain
            },
        ] {
            let columns = [plain, plain, odd, plain];
            let store = Store::new(&columns, SURFACES);
            assert!(matches!(store, Store::Wide(_)), "{odd:?}");
            assert_eq!(unpacked(&store), columns);
        }
        // The edges of the packed range, and of `i32`.
        let edges = [
            plain,
            Column {
                height: 100 + 32_767,
                ..plain
            },
            Column {
                height: 100 - 32_768,
                surface_depth: 63,
                soil_depth: 255,
                ..plain
            },
        ];
        let store = Store::new(&edges, SURFACES);
        assert!(matches!(store, Store::Packed(..)));
        assert_eq!(unpacked(&store), edges);
        for height in [i32::MIN, i32::MIN + 7, i32::MAX] {
            let one = [Column { height, ..plain }];
            let store = Store::new(&one, SURFACES);
            let Store::Packed(packing, words) = &store else {
                panic!("{height} packs");
            };
            assert_eq!(packing.bedrock_below(words), one[0].bedrock_below());
            assert_eq!(unpacked(&store), one);
        }
        assert_eq!(Store::new(&[], SURFACES).len(), 0);
    }

    /// The shipped terrain spans about 22,500 voxels (352 m) in all, so every
    /// footprint packs whatever its first column.
    #[test]
    fn shipped_terrain_footprints_pack() {
        let g =
            Generator::new(0x5EED, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap();
        let bounds = g.height_bounds();
        assert!(
            bounds.max - bounds.min < 1 << (HEIGHT_BITS - 1),
            "{bounds:?}"
        );
        let [thin, thick] = g.config().surface.grass_depth_voxels;
        assert!(thin <= thick && thick <= MAX_COVER);
        let columns = g.columns(ChunkCoord::new(517, 512, 509).unwrap());
        assert!(columns.is_packed());
        assert_eq!(columns.heap_bytes(), 4 << 20);
        let all = columns.to_vec();
        assert_eq!(Store::new(&all, g.stages.surface.surfaces()), columns.store);
        assert_eq!(
            columns.bedrock_below(),
            all.iter().map(Column::bedrock_below).min().unwrap()
        );
        let lo = all.iter().map(|c| c.height).min().unwrap();
        let hi = all.iter().map(|c| c.height).max().unwrap();
        assert_eq!(columns.height_range(), (lo, hi));
    }

    /// Grass that can be thicker than 63 voxels does not pack: the
    /// generator keeps that terrain's footprints wide, with the same columns
    /// as its point queries.
    #[test]
    fn thick_grass_keeps_footprints_wide() {
        let mut config = TerrainConfig::default();
        config.surface.grass_depth_voxels = [60, 70];
        let g = Generator::new(0x5EED, GeneratorVersion::CURRENT, config).unwrap();
        let chunk = ChunkCoord::new(517, 512, 509).unwrap();
        let columns = g.columns(chunk);
        assert!(!columns.is_packed());
        assert_eq!(columns.heap_bytes(), 8 << 20);
        let min = chunk.min_voxel();
        for (x, z) in [(0, 0), (1023, 0), (400, 700), (1023, 1023)] {
            let want = g.column(min.x + x as i32, min.z + z as i32).unwrap();
            assert_eq!(columns.column(x, z), Some(want), "({x}, {z})");
        }
        let all = columns.to_vec();
        let grass = g.config().surface.grass;
        for c in all.iter().filter(|c| c.surface == grass) {
            assert!((60..=70).contains(&c.surface_depth), "{c:?}");
        }
        assert_eq!(
            columns.bedrock_below(),
            all.iter().map(Column::bedrock_below).min().unwrap()
        );
        let lo = all.iter().map(|c| c.height).min().unwrap();
        let hi = all.iter().map(|c| c.height).max().unwrap();
        assert_eq!(columns.height_range(), (lo, hi));
    }
}
