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
//! Other cubes are answered by scanning their columns. The pyramid is built
//! on the first question the chunk's highest column cannot answer, so an
//! all-air chunk never builds it.
//!
//! With caves (generator version 2), a cube the heightfield calls solid is
//! also checked against the chunk's [`CaveGrid`]: untouched cubes keep their
//! answer, cubes carved entirely are air, and the rest are refined. Leaves
//! carve their solid voxels from the eight corners of their cell (a leaf
//! never spans two cells), sharing the interpolation work between voxels
//! ([`block_field`]).

use super::columns::ChunkColumns;
use super::coord::CHUNK_SIZE;
use crate::regional::caves::{CAVE_CELL, CaveGrid, Caves, CubeCaves, block_field, trilinear};
use crate::surface::Column;
use std::sync::{Arc, OnceLock};
use world_core::{MaterialId, RegionSource, VoxelPos};

/// Footprint summary: the highest column and the lowest bedrock boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    min_height: i32,
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
        min_height: i32::MAX,
        max_height: i32::MIN,
        min_bedrock: i32::MAX,
        min_soil_end: i32::MAX,
        max_soil_start: i32::MIN,
    };

    #[inline]
    fn of(column: &Column) -> Self {
        // Soil is depth surface_depth..soil_depth below the top voxel; with
        // no soil (soil_depth <= surface_depth) the range is empty. Both
        // differences saturate, like `Column::bedrock_below` (which is the
        // soil start).
        let height = column.height;
        let soil_end = height.saturating_sub(i32::from(column.surface_depth));
        let soil_start =
            height.saturating_sub(i32::from(column.soil_depth.max(column.surface_depth)));
        Self {
            min_height: height,
            max_height: height,
            min_bedrock: soil_start,
            min_soil_end: soil_end,
            max_soil_start: soil_start,
        }
    }

    #[inline]
    fn merge(self, other: Self) -> Self {
        Self {
            min_height: self.min_height.min(other.min_height),
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
    /// `1..=5`, `x` fastest. Built on first use.
    pyramid: OnceLock<Vec<Vec<Span>>>,
    /// The cave fields and this chunk's grid (`None` inside when no voxel
    /// of the chunk can be carved).
    caves: Option<(Arc<Caves>, Option<CaveGrid>)>,
}

impl ChunkSource {
    /// The heightfield alone.
    pub fn new(columns: ChunkColumns) -> Self {
        Self::with_caves(columns, None)
    }

    /// The heightfield with `caves` carved out of it; computes the chunk's
    /// cave grid when any voxel can be carved.
    pub fn with_caves(columns: ChunkColumns, caves: Option<Arc<Caves>>) -> Self {
        let caves = caves.map(|caves| {
            let grid = caves.grid(columns.chunk(), columns.height_range().1);
            (caves, grid)
        });
        Self {
            columns,
            pyramid: OnceLock::new(),
            caves,
        }
    }

    /// Whether this source carves caves.
    pub fn has_caves(&self) -> bool {
        self.caves.is_some()
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
        let (dx, dz) = self.local(min, size)?;
        let bottom = i64::from(min.y);
        let top = bottom + i64::from(size) - 1;
        // Above the chunk's highest column: air, without the pyramid.
        if bottom >= i64::from(self.columns.height_range().1) {
            return Some(MaterialId::AIR);
        }
        let span = self.footprint(dx, dz, size);
        if bottom >= i64::from(span.max_height) {
            return Some(MaterialId::AIR);
        }
        let caves = match &self.caves {
            Some((caves, grid)) => caves.cube(
                self.columns.chunk(),
                grid.as_ref(),
                min,
                size,
                (span.min_height, span.max_height),
            ),
            None => CubeCaves::None,
        };
        match caves {
            CubeCaves::All => return Some(MaterialId::AIR),
            CubeCaves::Mixed => return None,
            CubeCaves::None => {}
        }
        if top < i64::from(span.min_bedrock) {
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
        let local_x = i64::from(min.x) - i64::from(chunk_min.x);
        let local_z = i64::from(min.z) - i64::from(chunk_min.z);
        let inside = 0..=i64::from(CHUNK_SIZE) - 4;
        if !(inside.contains(&local_x) && inside.contains(&local_z)) {
            return self.leaf_at_edge(min, voxels);
        }
        let layers = self.columns.layers();
        let carving = self.leaf_caves(min);
        // Inside one cell the 64 field values share their interpolation.
        let field = carving
            .as_ref()
            .and_then(|c| c.corners)
            .map(|corners| block_field(&corners, min, [0, 1, 2, 3]));
        let ys: [i64; 4] = std::array::from_fn(|y| i64::from(min.y.wrapping_add(y as i32)));
        let n = CHUNK_SIZE as usize;
        let first = local_z as usize * n + local_x as usize;
        for z in 0..4 {
            let row = &self.columns.columns()[first + z * n..][..4];
            for (x, column) in row.iter().enumerate() {
                // `Column::material_at` as three thresholds on `y`.
                let height = i64::from(column.height);
                let cover_from = height - i64::from(column.surface_depth);
                let soil_from = height - i64::from(column.soil_depth);
                let carvable = carving.as_ref().map(|c| c.caves.carvable_range(column));
                for (y, &at) in ys.iter().enumerate() {
                    let mut material = if at >= height {
                        MaterialId::AIR
                    } else if at >= cover_from {
                        column.surface
                    } else if at >= soil_from {
                        layers.soil
                    } else {
                        layers.bedrock
                    };
                    if let (Some(carving), Some((lo, hi))) = (&carving, carvable)
                        && !material.is_air()
                        && lo <= at
                        && at < hi
                    {
                        let carved = match &field {
                            Some(f) => f[z][y][x] < 0.0,
                            None => {
                                let pos = VoxelPos::new(
                                    min.x.wrapping_add(x as i32),
                                    min.y.wrapping_add(y as i32),
                                    min.z.wrapping_add(z as i32),
                                );
                                carving.caves.carved(pos, column)
                            }
                        };
                        if carved {
                            material = MaterialId::AIR;
                        }
                    }
                    voxels[x + 4 * y + 16 * z] = material;
                }
            }
        }
    }

    /// [`leaf`](Self::leaf) for a leaf not wholly inside the footprint, one
    /// voxel at a time.
    fn leaf_at_edge(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        let chunk_min = self.columns.chunk().min_voxel();
        let layers = self.columns.layers();
        let carving = self.leaf_caves(min);
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
                if let (Some(column), Some(carving)) = (column, &carving) {
                    carving.carve_column(column, min, x, z, voxels);
                }
            }
        }
    }

    /// How the leaf at `min` is carved: `None` when no voxel can be.
    fn leaf_caves(&self, min: VoxelPos) -> Option<LeafCaves<'_>> {
        let (caves, grid) = self.caves.as_ref()?;
        let chunk_min = self.columns.chunk().min_voxel();
        let inside = |v: i32, o: i32| {
            let d = i64::from(v) - i64::from(o);
            d >= 0 && d + 4 <= i64::from(CHUNK_SIZE)
        };
        let one_cell = |v: i32| (v & (CAVE_CELL - 1)) <= CAVE_CELL - 4;
        let in_chunk = inside(min.x, chunk_min.x)
            && inside(min.y, chunk_min.y)
            && inside(min.z, chunk_min.z)
            && one_cell(min.x)
            && one_cell(min.y)
            && one_cell(min.z);
        // A leaf inside the chunk and one cell (every leaf the builder asks
        // for) takes the grid's corners for all its voxels; any other leaf
        // computes each voxel on its own.
        let corners = if in_chunk {
            let grid = grid.as_ref()?;
            if grid.cell_bounds(min).0 >= 0.0 {
                return None;
            }
            Some(grid.corners(min)?)
        } else {
            None
        };
        Some(LeafCaves { caves, corners })
    }

    /// The chunk-local corner of the footprint `[min.x, min.x + size) x
    /// [min.z, ...)`, or `None` if it is empty or leaves the chunk.
    #[inline]
    fn local(&self, min: VoxelPos, size: u32) -> Option<(usize, usize)> {
        let chunk_min = self.columns.chunk().min_voxel();
        let dx = i64::from(min.x) - i64::from(chunk_min.x);
        let dz = i64::from(min.z) - i64::from(chunk_min.z);
        let n = i64::from(CHUNK_SIZE);
        let s = i64::from(size);
        if size == 0 || dx < 0 || dz < 0 || dx + s > n || dz + s > n {
            return None;
        }
        Some((dx as usize, dz as usize))
    }

    /// The summary of the footprint of side `size` at chunk-local `(dx, dz)`,
    /// which [`local`](Self::local) accepted.
    #[inline]
    fn footprint(&self, dx: usize, dz: usize, size: u32) -> Span {
        let s = size as usize;
        // An aligned power-of-four square is one pyramid entry.
        if size.is_power_of_two()
            && size.trailing_zeros().is_multiple_of(2)
            && dx.is_multiple_of(s)
            && dz.is_multiple_of(s)
        {
            let k = size.trailing_zeros() as usize / 2;
            if k == 0 {
                return Span::of(&self.columns.columns()[dz * CHUNK_SIZE as usize + dx]);
            }
            let m = CHUNK_SIZE as usize >> (2 * k);
            return self.pyramid()[k - 1][(dz / s) * m + dx / s];
        }
        let mut span = Span::EMPTY;
        for z in dz..dz + s {
            for x in dx..dx + s {
                let column = self.columns.columns()[z * CHUNK_SIZE as usize + x];
                span = span.merge(Span::of(&column));
            }
        }
        span
    }

    /// The footprint pyramid, built on first use.
    fn pyramid(&self) -> &[Vec<Span>] {
        self.pyramid
            .get_or_init(|| build_pyramid(self.columns.columns()))
    }
}

/// The min/max pyramid over a chunk's columns (`x` fastest): level 1 straight
/// from the columns, each further level from the one below, a 4x4 block of
/// entries at a time.
fn build_pyramid(columns: &[Column]) -> Vec<Vec<Span>> {
    let mut n = CHUNK_SIZE as usize;
    let m = n / 4;
    let mut level = Vec::with_capacity(m * m);
    for bz in 0..m {
        let rows: [&[Column]; 4] = std::array::from_fn(|z| &columns[(4 * bz + z) * n..][..n]);
        for bx in 0..m {
            let mut span = Span::EMPTY;
            for row in rows {
                for column in &row[4 * bx..4 * bx + 4] {
                    span = span.merge(Span::of(column));
                }
            }
            level.push(span);
        }
    }
    let mut pyramid = vec![level];
    n = m;
    while n > 1 {
        let m = n / 4;
        let below = &pyramid[pyramid.len() - 1];
        let mut level = Vec::with_capacity(m * m);
        for bz in 0..m {
            for bx in 0..m {
                let mut span = Span::EMPTY;
                for z in 0..4 {
                    for &entry in &below[(4 * bz + z) * n + 4 * bx..][..4] {
                        span = span.merge(entry);
                    }
                }
                level.push(span);
            }
        }
        pyramid.push(level);
        n = m;
    }
    pyramid
}

/// The cave context of one leaf.
struct LeafCaves<'a> {
    caves: &'a Caves,
    /// The corners of the leaf's cell, or `None` to compute per voxel.
    corners: Option<[f32; 8]>,
}

impl LeafCaves<'_> {
    /// Carves the cave voxels of column `(x, z)` of the leaf at `min`.
    fn carve_column(
        &self,
        column: &Column,
        min: VoxelPos,
        x: i32,
        z: i32,
        voxels: &mut [MaterialId; 64],
    ) {
        for y in 0..4 {
            let slot = (x + 4 * y + 16 * z) as usize;
            if voxels[slot].is_air() {
                continue;
            }
            let pos = VoxelPos::new(
                min.x.wrapping_add(x),
                min.y.wrapping_add(y),
                min.z.wrapping_add(z),
            );
            let carved = match &self.corners {
                Some(c) => self.caves.carvable(pos.y, column) && trilinear(c, pos) < 0.0,
                None => self.caves.carved(pos, column),
            };
            if carved {
                voxels[slot] = MaterialId::AIR;
            }
        }
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

#[cfg(test)]
mod tests {
    //! The fast paths against the plain ones: leaves and uniform answers
    //! against `Generator::material` voxel by voxel, the pyramid against
    //! column scans, and whole built chunks against a reference source that
    //! answers the way this module did before it had fast paths.

    use super::*;
    use crate::{ChunkCoord, Generator, GeneratorVersion, TerrainConfig};
    use proptest::prelude::*;
    use proptest::test_runner::Config;
    use world_core::VoxelTree;
    use world_core::tree::{CHUNK_LEVEL, same_content};
    use world_testkit::profile::scale;

    /// The kinds of chunk a column of chunks holds, top down.
    const KINDS: [&str; 6] = [
        "air",
        "surface",
        "just_below",
        "underground",
        "deep_underground",
        "below_caves",
    ];

    fn generator(seed: u64) -> Generator {
        Generator::new(seed, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap()
    }

    /// The chunk of `kind` in the column of chunks at `(x, z)`.
    fn chunk_of_kind(g: &Generator, x: u32, z: u32, kind: usize) -> ChunkCoord {
        let probe = g.columns(ChunkCoord::new(x, 512, z).unwrap());
        let (lo, hi) = probe.height_range();
        let min = probe.chunk().min_voxel();
        let at = |y: i32| ChunkCoord::containing(VoxelPos::new(min.x, y, min.z)).unwrap();
        let surface = at(lo + (hi - lo) / 2);
        let below = |d: u32| ChunkCoord::new(x, surface.y() - d, z).unwrap();
        match KINDS[kind] {
            "air" => at(hi + CHUNK_SIZE as i32),
            "surface" => surface,
            "just_below" => below(1),
            "underground" => below(3),
            "deep_underground" => below(12),
            // Under the cave band's floor (-256 m by default).
            _ => at(-300 * 64),
        }
    }

    /// The material of every voxel of the leaf at `min`, voxel by voxel.
    fn leaf_by_voxels(g: &Generator, min: VoxelPos) -> [MaterialId; 64] {
        std::array::from_fn(|slot| {
            let (x, y, z) = (slot as i32 & 3, (slot as i32 >> 2) & 3, slot as i32 >> 4);
            g.material(VoxelPos::new(min.x + x, min.y + y, min.z + z))
                .unwrap()
        })
    }

    /// A footprint summary by scanning its columns.
    fn scan(source: &ChunkSource, dx: usize, dz: usize, size: u32) -> Span {
        let n = CHUNK_SIZE as usize;
        let mut span = Span::EMPTY;
        for z in dz..dz + size as usize {
            for x in dx..dx + size as usize {
                span = span.merge(Span::of(&source.columns.columns()[z * n + x]));
            }
        }
        span
    }

    /// `Span::of` as first written, with 64-bit arithmetic and clamping.
    fn span_of_wide(column: &Column) -> Span {
        let height = i64::from(column.height);
        let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let soil_start = clamp(height - i64::from(column.soil_depth.max(column.surface_depth)));
        Span {
            min_height: column.height,
            max_height: column.height,
            min_bedrock: column.bedrock_below(),
            min_soil_end: clamp(height - i64::from(column.surface_depth)),
            max_soil_start: soil_start,
        }
    }

    /// The source as it answered before its fast paths: small footprints
    /// scanned, caves bounded by whole cells only, and every leaf voxel by
    /// voxel ([`ChunkSource::leaf_at_edge`] is the original leaf code).
    struct Reference<'a>(&'a ChunkSource);

    impl RegionSource for Reference<'_> {
        fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
            let source = self.0;
            let (dx, dz) = source.local(min, size)?;
            let span = if size >= 64 {
                source.footprint(dx, dz, size)
            } else {
                scan(source, dx, dz, size)
            };
            let bottom = i64::from(min.y);
            let top = bottom + i64::from(size) - 1;
            if bottom >= i64::from(span.max_height) {
                return Some(MaterialId::AIR);
            }
            if let Some((caves, grid)) = &source.caves {
                let heights = (span.min_height, span.max_height);
                match caves.cube_by_cells(source.columns.chunk(), grid.as_ref(), min, size, heights)
                {
                    CubeCaves::All => return Some(MaterialId::AIR),
                    CubeCaves::Mixed => return None,
                    CubeCaves::None => {}
                }
            }
            let layers = source.columns.layers();
            if top < i64::from(span.min_bedrock) {
                Some(layers.bedrock)
            } else if bottom >= i64::from(span.max_soil_start) && top < i64::from(span.min_soil_end)
            {
                Some(layers.soil)
            } else {
                None
            }
        }

        fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
            self.0.leaf_at_edge(min, voxels);
        }
    }

    fn build(at: ChunkCoord, source: &impl RegionSource) -> VoxelTree {
        let mut tree = VoxelTree::new();
        tree.build_region(CHUNK_LEVEL, at.min_voxel(), source)
            .unwrap();
        tree
    }

    #[test]
    fn spans_saturate_like_the_wide_arithmetic() {
        let extremes = [i32::MIN, i32::MIN + 3, -1, 0, 7, i32::MAX - 2, i32::MAX];
        for height in extremes {
            for (surface_depth, soil_depth) in [(0, 0), (5, 0), (2, 9), (255, 255), (3, 200)] {
                let column = Column {
                    height,
                    surface: MaterialId(3),
                    surface_depth,
                    soil_depth,
                };
                assert_eq!(Span::of(&column), span_of_wide(&column), "{column:?}");
            }
        }
    }

    #[test]
    fn an_air_chunk_never_builds_its_pyramid() {
        let g = generator(0x5EED);
        let air = chunk_of_kind(&g, 517, 509, 0);
        let source = g.chunk_source(air);
        let tree = build(air, &source);
        assert_eq!(tree.stats().chunks, 0);
        assert!(source.pyramid.get().is_none());
    }

    proptest! {
        #![proptest_config(Config::with_cases((scale().cases / 64).max(2)))]

        /// Every kind of chunk builds into the same tree through the fast
        /// paths as through the reference answers.
        #[test]
        fn chunks_build_as_the_reference_builds_them(
            seed in prop_oneof![Just(0x5EED_u64), any::<u64>()],
            x in 0u32..1024,
            z in 0u32..1024,
        ) {
            let g = generator(seed);
            for (kind, name) in KINDS.iter().enumerate() {
                let at = chunk_of_kind(&g, x, z, kind);
                let source = g.chunk_source(at);
                let fast = build(at, &source);
                let reference = build(at, &Reference(&source));
                prop_assert!(
                    same_content(&fast, &reference),
                    "{} chunk {:?} of seed {:#x}", name, at, seed
                );
            }
            // The heightfield alone, from the same columns.
            let at = chunk_of_kind(&g, x, z, 1);
            let source = ChunkSource::new(g.columns(at));
            prop_assert!(same_content(&build(at, &source), &build(at, &Reference(&source))));
        }

    }

    proptest! {
        #![proptest_config(world_testkit::profile::proptest_config())]

        /// Pyramid entries equal scans of their footprints.
        #[test]
        fn pyramid_entries_are_column_scans(
            k in 1u32..=5,
            i in 0usize..1024,
            j in 0usize..1024,
        ) {
            let source = &sites()[1].source;
            let size = 1u32 << (2 * k);
            let s = size as usize;
            let (dx, dz) = (i / s * s, j / s * s);
            prop_assert_eq!(source.footprint(dx, dz, size), scan(source, dx, dz, size));
        }

        /// Leaves at mixed places and anywhere in a chunk's footprint, at
        /// any height and alignment, equal the generator voxel by voxel.
        #[test]
        fn leaves_equal_the_generator(
            kind in 0usize..KINDS.len(),
            near in proptest::option::weighted(0.8, any::<usize>()),
            anywhere in (0i32..1024, -40i32..1064, 0i32..1024),
            jitter in prop_oneof![3 => Just((0, 0, 0)), 1 => (-15i32..16, -15i32..16, -15i32..16)],
            aligned in proptest::bool::weighted(0.7),
        ) {
            let site = &sites()[kind];
            let mut min = clamp_footprint(site, pick(site, near, anywhere, jitter), 4);
            if aligned {
                min = VoxelPos::new(min.x & !3, min.y & !3, min.z & !3);
            }
            let mut voxels = [MaterialId(9999); 64];
            site.source.leaf(min, &mut voxels);
            prop_assert_eq!(
                voxels,
                leaf_by_voxels(&site.generator, min),
                "{} leaf at {:?}", KINDS[kind], min
            );
        }

        /// Uniform answers for cubes of every tree size, aligned or not, at
        /// mixed places and anywhere, hold at the cube's corners and at
        /// random voxels inside; a leaf-size answer holds at every voxel.
        #[test]
        fn uniform_answers_equal_the_generator(
            kind in 0usize..KINDS.len(),
            level in prop_oneof![3 => Just(1u32), 2 => Just(2u32), 1 => 3u32..=5],
            near in proptest::option::weighted(0.8, any::<usize>()),
            anywhere in (0i32..1024, -40i32..1064, 0i32..1024),
            jitter in prop_oneof![3 => Just((0, 0, 0)), 1 => (-15i32..16, -15i32..16, -15i32..16)],
            aligned in proptest::bool::weighted(0.7),
            probes in proptest::collection::vec((0u32..1024, 0u32..1024, 0u32..1024), 24),
        ) {
            let site = &sites()[kind];
            let g = &site.generator;
            let size = 1u32 << (2 * level);
            let mut min = pick(site, near, anywhere, jitter);
            if aligned {
                let mask = !(size as i32 - 1);
                min = VoxelPos::new(min.x & mask, min.y & mask, min.z & mask);
            }
            let min = clamp_footprint(site, min, size);
            if let Some(m) = site.source.uniform(min, size) {
                let last = size - 1;
                let corners = (0..8).map(|n| (last * (n & 1), last * (n >> 1 & 1), last * (n >> 2)));
                let inside = probes.iter().map(|&(a, b, c)| (a % size, b % size, c % size));
                for (a, b, c) in corners.chain(inside) {
                    let p = VoxelPos::new(min.x + a as i32, min.y + b as i32, min.z + c as i32);
                    prop_assert_eq!(g.material(p).unwrap(), m, "{} cube {:?} {}", KINDS[kind], min, size);
                }
                if size == 4 {
                    prop_assert_eq!(leaf_by_voxels(g, min), [m; 64]);
                }
            }
        }
    }

    /// One chunk of each kind for seed 0x5EED.
    struct Site {
        generator: Generator,
        source: ChunkSource,
        /// Every mixed leaf of the chunk, found by the reference build:
        /// the surface, layer boundaries and cave walls.
        mixed: Vec<VoxelPos>,
    }

    /// Records where the reference reads leaves.
    struct Recording<'a>(Reference<'a>, std::cell::RefCell<Vec<VoxelPos>>);

    impl RegionSource for Recording<'_> {
        fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
            self.0.uniform(min, size)
        }

        fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
            self.0.leaf(min, voxels);
            if voxels.iter().any(|&m| m != voxels[0]) {
                self.1.borrow_mut().push(min);
            }
        }
    }

    fn sites() -> &'static [Site] {
        static SITES: std::sync::OnceLock<Vec<Site>> = std::sync::OnceLock::new();
        SITES.get_or_init(|| {
            (0..KINDS.len())
                .map(|kind| {
                    let generator = generator(0x5EED);
                    let at = chunk_of_kind(&generator, 517, 509, kind);
                    let source = generator.chunk_source(at);
                    let recording = Recording(Reference(&source), Default::default());
                    build(at, &recording);
                    let mixed = recording.1.into_inner();
                    Site {
                        generator,
                        source,
                        mixed,
                    }
                })
                .collect()
        })
    }

    /// A voxel of `site`: near a mixed leaf when `near` picks one (any
    /// leaf when it has none), else anywhere in the chunk; `jitter` moves it
    /// up to 15 voxels on each axis.
    fn pick(
        site: &Site,
        near: Option<usize>,
        anywhere: (i32, i32, i32),
        jitter: (i32, i32, i32),
    ) -> VoxelPos {
        let chunk_min = site.source.columns().chunk().min_voxel();
        let base = match near {
            Some(i) if !site.mixed.is_empty() => site.mixed[i % site.mixed.len()],
            _ => VoxelPos::new(
                chunk_min.x + anywhere.0,
                chunk_min.y + anywhere.1,
                chunk_min.z + anywhere.2,
            ),
        };
        VoxelPos::new(base.x + jitter.0, base.y + jitter.1, base.z + jitter.2)
    }

    /// Moves `pos` so a cube of `size` there stays in the chunk's footprint.
    fn clamp_footprint(site: &Site, pos: VoxelPos, size: u32) -> VoxelPos {
        let chunk_min = site.source.columns().chunk().min_voxel();
        let fit = |v: i32, o: i32| v.clamp(o, o + (CHUNK_SIZE - size) as i32);
        VoxelPos::new(fit(pos.x, chunk_min.x), pos.y, fit(pos.z, chunk_min.z))
    }
}
