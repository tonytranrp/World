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
//! surface layers (grass, dirt, stone) by slope, altitude and moisture. See
//! the module docs for the order of operations.

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
    CoarseColumns, CoarseSource, MAX_COARSE_LEVEL,
};
pub use config::{HeightBounds, TerrainConfig};
pub use surface::{Column, Layers};

use chunk::Stages;
use continental::Continental;
use regional::Regional;
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

    /// The version new worlds are made with.
    pub const CURRENT: Self = Self::V1;

    /// Every version this build can generate.
    pub const SUPPORTED: &'static [Self] = &[Self::V1];
}

/// What produced a piece of output: the seed, the generator version and a
/// fingerprint of the parameters. Two outputs with equal ids for the same
/// chunk are identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GeneratorId {
    pub seed: u64,
    pub version: GeneratorVersion,
    /// [`TerrainConfig::fingerprint`] of the parameters.
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
        Ok(Self {
            id: GeneratorId {
                seed,
                version,
                config: config.fingerprint(),
            },
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

    /// Every column of `chunk`'s footprint (its `y` is ignored).
    pub fn columns(&self, chunk: ChunkCoord) -> ChunkColumns {
        self.stages.columns(self.id, chunk)
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
        Ok(self
            .stages
            .column(pos.x, pos.z)
            .material_at(pos.y, &self.layers()))
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
        assert!(Generator::new(1, GeneratorVersion(2), config.clone()).is_err());
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

    #[test]
    fn generators_are_shareable() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Generator>();
        send_sync::<ChunkColumns>();
    }
}
