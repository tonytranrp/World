//! Chunk addressing.
//!
//! A chunk is the 1024³-voxel (16 m) subtree of decision 0006, the unit of
//! generation and residency. Its coordinate is the world-local voxel
//! coordinate (`pos - WORLD_MIN_VOXEL`) shifted right by 10 bits, so each axis
//! is `0..1024` and the chunk at `(512, 512, 512)` starts at the world origin.

use std::fmt;

use world_core::VoxelPos;
use world_core::consts::{WORLD_MIN_VOXEL, WORLD_SIZE_VOXELS};

/// log2 of the chunk edge in voxels.
pub const CHUNK_SHIFT: u32 = 10;

/// Voxels along one edge of a chunk: 1024, 16 m.
pub const CHUNK_SIZE: u32 = 1 << CHUNK_SHIFT;

/// Chunks along one edge of the world: 1024.
pub const CHUNKS_PER_AXIS: u32 = WORLD_SIZE_VOXELS >> CHUNK_SHIFT;

/// A chunk coordinate, each axis in `0..CHUNKS_PER_AXIS`. The fields are
/// private so a value always names a chunk inside the world.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkCoord {
    x: u32,
    y: u32,
    z: u32,
}

/// A chunk coordinate outside the world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkOutOfWorld {
    pub coord: [u32; 3],
}

impl fmt::Display for ChunkOutOfWorld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "chunk {:?} is outside the world (each axis is 0 to {})",
            self.coord,
            CHUNKS_PER_AXIS - 1
        )
    }
}

impl std::error::Error for ChunkOutOfWorld {}

impl ChunkCoord {
    /// The chunk `(x, y, z)`, or an error if any axis is `CHUNKS_PER_AXIS` or
    /// more.
    pub const fn new(x: u32, y: u32, z: u32) -> Result<Self, ChunkOutOfWorld> {
        if x < CHUNKS_PER_AXIS && y < CHUNKS_PER_AXIS && z < CHUNKS_PER_AXIS {
            Ok(Self { x, y, z })
        } else {
            Err(ChunkOutOfWorld { coord: [x, y, z] })
        }
    }

    /// The chunk holding voxel `pos`, or `None` outside the world.
    pub const fn containing(pos: VoxelPos) -> Option<Self> {
        if !pos.in_world() {
            return None;
        }
        Some(Self {
            x: chunk_of(pos.x),
            y: chunk_of(pos.y),
            z: chunk_of(pos.z),
        })
    }

    pub const fn x(self) -> u32 {
        self.x
    }

    pub const fn y(self) -> u32 {
        self.y
    }

    pub const fn z(self) -> u32 {
        self.z
    }

    pub const fn to_array(self) -> [u32; 3] {
        [self.x, self.y, self.z]
    }

    /// The chunk's lowest voxel.
    pub const fn min_voxel(self) -> VoxelPos {
        VoxelPos::new(
            chunk_start(self.x),
            chunk_start(self.y),
            chunk_start(self.z),
        )
    }
}

/// The chunk index of a world voxel coordinate inside the world.
const fn chunk_of(v: i32) -> u32 {
    ((v - WORLD_MIN_VOXEL) as u32) >> CHUNK_SHIFT
}

/// The first world voxel coordinate of chunk index `c` (below 1024).
const fn chunk_start(c: u32) -> i32 {
    (c << CHUNK_SHIFT) as i32 + WORLD_MIN_VOXEL
}

impl fmt::Debug for ChunkCoord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChunkCoord({}, {}, {})", self.x, self.y, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::consts::WORLD_END_VOXEL;

    #[test]
    fn the_world_origin_is_the_corner_of_chunk_512() {
        let c = ChunkCoord::containing(VoxelPos::ORIGIN).unwrap();
        assert_eq!(c.to_array(), [512, 512, 512]);
        assert_eq!(c.min_voxel(), VoxelPos::ORIGIN);
        let below = ChunkCoord::containing(VoxelPos::new(-1, -1, -1)).unwrap();
        assert_eq!(below.to_array(), [511, 511, 511]);
        assert_eq!(below.min_voxel(), VoxelPos::new(-1024, -1024, -1024));
    }

    #[test]
    fn world_edges_are_the_first_and_last_chunks() {
        let lo = VoxelPos::new(WORLD_MIN_VOXEL, WORLD_MIN_VOXEL, WORLD_MIN_VOXEL);
        let hi = VoxelPos::new(WORLD_END_VOXEL - 1, WORLD_END_VOXEL - 1, 0);
        assert_eq!(ChunkCoord::containing(lo).unwrap().to_array(), [0, 0, 0]);
        assert_eq!(
            ChunkCoord::containing(hi).unwrap().to_array(),
            [1023, 1023, 512]
        );
        assert_eq!(
            ChunkCoord::containing(VoxelPos::new(WORLD_END_VOXEL, 0, 0)),
            None
        );
        assert!(ChunkCoord::new(1024, 0, 0).is_err());
        assert_eq!(ChunkCoord::new(0, 0, 0).unwrap().min_voxel(), lo);
    }
}
