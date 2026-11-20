//! World_Gen: deterministic procedural generation addressed by chunk.
//!
//! The same seed, generator version, parameters and chunk coordinate produce
//! identical voxels on every machine (decision 0008). This crate has no Bevy
//! dependency.
//!
//! ```
//! use world_gen::{ChunkCoord, Generator, GeneratorVersion, TerrainConfig};
//!
//! let generator = Generator::new(7, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap();
//! let chunk = ChunkCoord::new(512, 512, 512).unwrap();
//! let columns = generator.columns(chunk);
//! let top = columns.column(0, 0).unwrap().height;
//! assert!(columns.material(0, top, 0).unwrap().is_air());
//! ```
//!
//! Version 1 terrain is a heightfield: continental elevation, mountains and
//! hills on a one-metre lattice, regional relief on a 4-voxel lattice, and
//! surface layers (grass, dirt, stone) by slope, altitude and moisture.
//! Version 2 carves caves out of it ([`regional::caves`]). See the module
//! docs for the order of operations.

pub mod chunk;
pub mod config;
pub mod continental;
mod fingerprint;
pub mod noise;
pub mod regional;
pub mod surface;

use std::fmt;

pub use chunk::{
    CHUNK_SIZE, CHUNKS_PER_AXIS, ChunkColumns, ChunkCoord, ChunkOutOfWorld, ChunkSource,
    CoarseColumns, CoarseSource, FootprintBounds, MAX_COARSE_LEVEL,
};
pub use config::{HeightBounds, TerrainConfig};
pub use surface::{Column, Layers};

use chunk::Stages;
use continental::Continental;
use regional::Regional;
use regional::caves::Caves;
use std::sync::Arc;
use surface::Surface;
use world_core::{MaterialId, VoxelPos};

/// The version of the generation algorithm. Any change to generated output,
/// however small, is a new version; old versions stay in the code for as long
/// as saves made with them are supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GeneratorVersion(pub u32);

impl GeneratorVersion {
    /// Heightfield terrain: Perlin fBm continents, ridged mountains, hills,
    /// relief, and grass, dirt and stone layers.
    pub const V1: Self = Self(1);

    /// Version 1 with caves: tunnels and caverns from 3D noise, carved below
    /// the surface cover.
    pub const V2: Self = Self(2);

    /// The version new worlds are made with.
    pub const CURRENT: Self = Self::V2;

    /// Every version this build can generate.
    pub const SUPPORTED: &'static [Self] = &[Self::V1, Self::V2];
}

/// What produced a piece of output: the seed, the generator version and a
/// fingerprint of the parameters. Two outputs with equal ids for the same
/// chunk are identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GeneratorId {
    pub seed: u64,
    pub version: GeneratorVersion,
    /// [`TerrainConfig::fingerprint_for`] the version.
    pub config: u64,
}

/// Why a generator or a query was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenError {
    /// This build cannot generate the requested version.
    UnsupportedVersion(GeneratorVersion),
    /// The parameters failed validation; every problem is listed.
    InvalidConfig(Vec<String>),
    /// A column or voxel outside the world.
    OutsideWorld { x: i32, y: i32, z: i32 },
    /// A coarse level outside `1..=MAX_COARSE_LEVEL`.
    BadCoarseLevel(u32),
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(
                f,
                "generator version {} is not supported (supported: {:?})",
                v.0,
                GeneratorVersion::SUPPORTED
                    .iter()
                    .map(|v| v.0)
                    .collect::<Vec<_>>()
            ),
            Self::InvalidConfig(problems) => {
                write!(f, "invalid terrain config: {}", problems.join("; "))
            }
            Self::OutsideWorld { x, y, z } => {
                write!(f, "({x}, {y}, {z}) is outside the world")
            }
            Self::BadCoarseLevel(level) => {
                write!(f, "coarse level {level} is not 1 to {MAX_COARSE_LEVEL}")
            }
        }
    }
}

impl std::error::Error for GenError {}

/// A terrain generator for one seed, version and set of parameters.
///
/// Building it validates the parameters and prepares every noise field;
/// queries never fail on a valid chunk and are pure functions of the
/// generator and the coordinate, so a `Generator` can be shared between
/// threads (`Send + Sync`) and asked in any order.
#[derive(Clone)]
pub struct Generator {
    id: GeneratorId,
    config: TerrainConfig,
    stages: Stages,
    /// The cave fields (version 2 with caves enabled).
    caves: Option<Arc<Caves>>,
}

impl Generator {
    /// A generator for `seed` and `version` with `config`, or why not.
    pub fn new(
        seed: u64,
        version: GeneratorVersion,
        config: TerrainConfig,
    ) -> Result<Self, GenError> {
        if !GeneratorVersion::SUPPORTED.contains(&version) {
            return Err(GenError::UnsupportedVersion(version));
        }
        let problems = config.problems();
        if !problems.is_empty() {
            return Err(GenError::InvalidConfig(problems));
        }
        let continental =
            Continental::new(seed, &config.continental).map_err(GenError::InvalidConfig)?;
        let stages = Stages {
            continental,
            regional: Regional::new(seed, &config.regional),
            surface: Surface::new(seed, &config.surface),
        };
        let caves = if version >= GeneratorVersion::V2 {
            Caves::new(seed, &config.caves).map(Arc::new)
        } else {
            None
        };
        Ok(Self {
            id: GeneratorId {
                seed,
                version,
                config: config.fingerprint_for(version),
            },
            caves,
            config,
            stages,
        })
    }

    /// The identity every output of this generator carries.
    pub fn id(&self) -> GeneratorId {
        self.id
    }

    pub fn config(&self) -> &TerrainConfig {
        &self.config
    }

    /// Bounds on every column height this generator can produce.
    pub fn height_bounds(&self) -> HeightBounds {
        self.config.height_bounds()
    }

    /// The soil and bedrock materials under column covers.
    pub fn layers(&self) -> Layers {
        self.stages.surface.layers()
    }

    /// Every column of `chunk`'s footprint (its `y` is ignored). Columns are
    /// the heightfield alone; [`chunk_source`](Self::chunk_source) adds the
    /// caves.
    pub fn columns(&self, chunk: ChunkCoord) -> ChunkColumns {
        self.stages.columns(self.id, chunk)
    }

    /// Everything needed to build `chunk` into a tree: its columns and, from
    /// version 2, its cave grid. Its voxels equal [`material`](Self::material).
    pub fn chunk_source(&self, chunk: ChunkCoord) -> ChunkSource {
        ChunkSource::with_caves(self.columns(chunk), self.caves.clone())
    }

    /// [`chunk_source`](Self::chunk_source) for `chunk` from `columns`
    /// computed for any chunk stacked on the same footprint: columns do not
    /// depend on `y`, so this skips the heightfield, most of the cost of a
    /// chunk. `None` when `columns` came from another generator or
    /// footprint.
    pub fn chunk_source_from(
        &self,
        columns: &ChunkColumns,
        chunk: ChunkCoord,
    ) -> Option<ChunkSource> {
        if !columns.serves(self.id, chunk) {
            return None;
        }
        let columns = columns.for_chunk(chunk)?;
        Some(ChunkSource::with_caves(columns, self.caves.clone()))
    }

    /// The one material of every voxel of `chunk`, when its footprint's
    /// `bounds` prove it without building: air when the chunk lies at or
    /// above every column's top (caves only carve solid voxels), bedrock
    /// when it lies below every column's bedrock and no cave can reach it.
    /// `None` when not proven, or when `bounds` belong to another generator
    /// or footprint. The chunk built from
    /// [`chunk_source`](Self::chunk_source) is that material wherever this
    /// answers.
    pub fn uniform_chunk(&self, bounds: &FootprintBounds, chunk: ChunkCoord) -> Option<MaterialId> {
        if !bounds.serves(self.id, chunk) {
            return None;
        }
        let bottom = i64::from(chunk.min_voxel().y);
        let top = bottom + i64::from(CHUNK_SIZE); // exclusive
        let (_, max_height) = bounds.height_range;
        if bottom >= i64::from(max_height) {
            return Some(MaterialId::AIR);
        }
        if top > i64::from(bounds.bedrock_below) {
            return None;
        }
        // A voxel can be carved only where some column allows it, and the
        // highest column allows the most (the cover is measured from the
        // top), so probing it at every height of the chunk bounds them all.
        if let Some(caves) = &self.caves {
            let highest = Column {
                height: max_height,
                surface: MaterialId::AIR,
                surface_depth: 0,
                soil_depth: 0,
            };
            if (bottom..top).any(|y| caves.carvable(y as i32, &highest)) {
                return None;
            }
        }
        Some(self.layers().bedrock)
    }

    /// Whether this generator carves caves.
    pub fn has_caves(&self) -> bool {
        self.caves.is_some()
    }

    /// `chunk`'s footprint sampled every `4^level` voxels, `level` in
    /// `1..=MAX_COARSE_LEVEL`. Each sample equals the full-resolution column
    /// at the same place.
    pub fn coarse_columns(&self, chunk: ChunkCoord, level: u32) -> Result<CoarseColumns, GenError> {
        if !(1..=MAX_COARSE_LEVEL).contains(&level) {
            return Err(GenError::BadCoarseLevel(level));
        }
        Ok(self.stages.coarse(self.id, chunk, level))
    }

    /// One world column, equal to the column a chunk footprint has there.
    /// Slower per column than [`columns`](Self::columns), for point queries.
    pub fn column(&self, x: i32, z: i32) -> Result<Column, GenError> {
        if !VoxelPos::new(x, 0, z).in_world() {
            return Err(GenError::OutsideWorld { x, y: 0, z });
        }
        Ok(self.stages.column(x, z))
    }

    /// The material of one voxel. A point query; use [`columns`](Self::columns)
    /// to fill chunks.
    pub fn material(&self, pos: VoxelPos) -> Result<MaterialId, GenError> {
        if !pos.in_world() {
            return Err(GenError::OutsideWorld {
                x: pos.x,
                y: pos.y,
                z: pos.z,
            });
        }
        let column = self.stages.column(pos.x, pos.z);
        let material = column.material_at(pos.y, &self.layers());
        match &self.caves {
            Some(caves) if !material.is_air() && caves.carved(pos, &column) => Ok(MaterialId::AIR),
            _ => Ok(material),
        }
    }
}

impl fmt::Debug for Generator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Generator")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_versions_and_bad_configs_are_refused() {
        let config = TerrainConfig::default();
        assert_eq!(
            Generator::new(1, GeneratorVersion(0), config.clone()).unwrap_err(),
            GenError::UnsupportedVersion(GeneratorVersion(0))
        );
        assert!(Generator::new(1, GeneratorVersion(3), config.clone()).is_err());
        let mut bad = config;
        bad.regional.relief_height_m = f32::NAN;
        assert!(matches!(
            Generator::new(1, GeneratorVersion::CURRENT, bad),
            Err(GenError::InvalidConfig(p)) if p.len() == 1
        ));
    }

    #[test]
    fn queries_outside_the_world_are_errors() {
        let g = Generator::new(1, GeneratorVersion::CURRENT, TerrainConfig::default()).unwrap();
        assert!(g.column(world_core::consts::WORLD_END_VOXEL, 0).is_err());
        assert!(g.material(VoxelPos::new(0, i32::MIN, 0)).is_err());
        assert_eq!(
            g.coarse_columns(ChunkCoord::new(0, 0, 0).unwrap(), 0),
            Err(GenError::BadCoarseLevel(0))
        );
        assert!(
            g.coarse_columns(ChunkCoord::new(0, 0, 0).unwrap(), 6)
                .is_err()
        );
    }

    /// The content hash of `chunk` built from `source`, and its content when
    /// it is one material.
    fn built(chunk: ChunkCoord, source: &ChunkSource) -> (u64, Option<MaterialId>) {
        let mut tree = world_core::VoxelTree::new();
        let min = chunk.min_voxel();
        tree.build_region(world_core::tree::CHUNK_LEVEL, min, source)
            .unwrap();
        let hash = world_core::serialize::content_hash(&tree);
        (hash, tree.take_chunk(min).unwrap().uniform())
    }

    #[test]
    fn stacked_chunks_build_the_same_from_shared_columns() {
        for version in GeneratorVersion::SUPPORTED {
            let g = Generator::new(0xC0FFEE, *version, TerrainConfig::default()).unwrap();
            let probe = ChunkCoord::new(511, 512, 518).unwrap();
            let shared = g.columns(probe);
            let bounds = shared.bounds();
            let (lo, hi) = shared.height_range();
            let min = probe.min_voxel();
            let at = |y: i32| ChunkCoord::containing(VoxelPos::new(min.x, y, min.z)).unwrap();
            let surface = at(lo + (hi - lo) / 2);
            let air = at(hi + CHUNK_SIZE as i32);
            let buried = at(lo - 2 * CHUNK_SIZE as i32);
            // Below the cave band (-256 m).
            let deep = at(-20_000);
            for chunk in [at(hi), air, surface, buried, deep] {
                let fresh = g.chunk_source(chunk);
                let from = g.chunk_source_from(&shared, chunk).unwrap();
                assert_eq!(from.columns(), fresh.columns(), "{chunk:?}");
                let (want, content) = built(chunk, &fresh);
                assert_eq!(built(chunk, &from).0, want, "{version:?} {chunk:?}");
                if let Some(m) = g.uniform_chunk(&bounds, chunk) {
                    assert_eq!(content, Some(m), "{version:?} {chunk:?}");
                }
            }
            assert_eq!(g.uniform_chunk(&bounds, air), Some(MaterialId::AIR));
            assert_eq!(g.uniform_chunk(&bounds, surface), None);
            assert_eq!(g.uniform_chunk(&bounds, deep), Some(g.layers().bedrock));
            // Inside the cave band, rock is uniform only without caves.
            assert_eq!(
                g.uniform_chunk(&bounds, buried).is_some(),
                !g.has_caves(),
                "{version:?}"
            );
            // Columns and bounds of another footprint or generator serve
            // nothing.
            let beside = ChunkCoord::new(512, air.y(), 518).unwrap();
            assert!(g.chunk_source_from(&shared, beside).is_none());
            assert_eq!(g.uniform_chunk(&bounds, beside), None);
            let other = Generator::new(1, *version, TerrainConfig::default()).unwrap();
            assert!(other.chunk_source_from(&shared, air).is_none());
            assert_eq!(other.uniform_chunk(&bounds, air), None);
        }
    }

    #[test]
    fn generators_are_shareable() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Generator>();
        send_sync::<ChunkColumns>();
    }
}
