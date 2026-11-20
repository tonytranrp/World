//! Which chunks the player needs: full-detail chunks in a sphere, coarse
//! chunk columns in a disc, both nearest first.

use world_core::VoxelPos;
use world_core::consts::VOXELS_PER_METER;
use world_gen::{CHUNK_SIZE, CHUNKS_PER_AXIS, ChunkCoord};

/// The distance from `eye` to the centre of `chunk`, in voxels.
pub fn chunk_distance(eye: VoxelPos, chunk: ChunkCoord) -> f32 {
    let min = chunk.min_voxel();
    let half = (CHUNK_SIZE / 2) as f32;
    let d = |e: i32, m: i32| (m as f32 + half) - e as f32;
    let (dx, dy, dz) = (d(eye.x, min.x), d(eye.y, min.y), d(eye.z, min.z));
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// The horizontal distance from `eye` to the centre of chunk column
/// (`x`, `z`), in voxels.
pub fn column_distance(eye: VoxelPos, x: u32, z: u32) -> f32 {
    let probe = ChunkCoord::new(x, 0, z).map(|c| c.min_voxel());
    let Ok(min) = probe else {
        return f32::MAX;
    };
    let half = (CHUNK_SIZE / 2) as f32;
    let dx = min.x as f32 + half - eye.x as f32;
    let dz = min.z as f32 + half - eye.z as f32;
    (dx * dx + dz * dz).sqrt()
}

/// Metres to voxels.
pub fn voxels(metres: f32) -> f32 {
    metres * VOXELS_PER_METER as f32
}

/// The chunks within `radius_m` of `eye`, nearest first, ties in coordinate
/// order.
pub fn sphere(eye: VoxelPos, radius_m: f32) -> Vec<ChunkCoord> {
    let r = voxels(radius_m);
    let Some(centre) = ChunkCoord::containing(eye) else {
        return Vec::new();
    };
    let reach = (r / CHUNK_SIZE as f32).ceil() as i64 + 1;
    let mut out: Vec<(u32, ChunkCoord)> = Vec::new();
    let c = centre.to_array().map(i64::from);
    for dz in -reach..=reach {
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let at = [c[0] + dx, c[1] + dy, c[2] + dz];
                if at.iter().any(|&v| v < 0 || v >= i64::from(CHUNKS_PER_AXIS)) {
                    continue;
                }
                let Ok(chunk) = ChunkCoord::new(at[0] as u32, at[1] as u32, at[2] as u32) else {
                    continue;
                };
                let d = chunk_distance(eye, chunk);
                if d <= r {
                    out.push((d.to_bits(), chunk));
                }
            }
        }
    }
    // Distances are non-negative, so their bit patterns sort like the floats.
    out.sort_unstable();
    out.into_iter().map(|(_, c)| c).collect()
}

/// The chunk columns `(x, z)` within `radius_m` horizontally of `eye`,
/// nearest first.
pub fn disc(eye: VoxelPos, radius_m: f32) -> Vec<(u32, u32)> {
    let r = voxels(radius_m);
    let Some(centre) = ChunkCoord::containing(eye) else {
        return Vec::new();
    };
    let reach = (r / CHUNK_SIZE as f32).ceil() as i64 + 1;
    let mut out: Vec<(u32, (u32, u32))> = Vec::new();
    let (cx, cz) = (i64::from(centre.x()), i64::from(centre.z()));
    for dz in -reach..=reach {
        for dx in -reach..=reach {
            let (x, z) = (cx + dx, cz + dz);
            if x < 0 || z < 0 || x >= i64::from(CHUNKS_PER_AXIS) || z >= i64::from(CHUNKS_PER_AXIS)
            {
                continue;
            }
            let d = column_distance(eye, x as u32, z as u32);
            if d <= r {
                out.push((d.to_bits(), (x as u32, z as u32)));
            }
        }
    }
    out.sort_unstable();
    out.into_iter().map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_64_metre_sphere_holds_the_chunks_whose_centre_is_inside() {
        let eye = VoxelPos::new(512, 300, 512);
        let chunks = sphere(eye, 64.0);
        // 64 m is 4 chunks; the sphere of chunk centres has a few hundred.
        assert!((150..400).contains(&chunks.len()), "{}", chunks.len());
        assert_eq!(chunks[0], ChunkCoord::containing(eye).unwrap());
        for pair in chunks.windows(2) {
            assert!(chunk_distance(eye, pair[0]) <= chunk_distance(eye, pair[1]));
        }
        assert!(
            chunks
                .iter()
                .all(|&c| chunk_distance(eye, c) <= voxels(64.0))
        );
    }

    #[test]
    fn plans_stop_at_the_world_edge() {
        let corner = VoxelPos::new(
            world_core::consts::WORLD_MIN_VOXEL,
            0,
            world_core::consts::WORLD_MIN_VOXEL,
        );
        let chunks = sphere(corner, 64.0);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.x() < 8 && c.z() < 8));
        let columns = disc(corner, 100.0);
        assert!(columns.iter().all(|&(x, z)| x < 10 && z < 10));
        assert_eq!(columns[0], (0, 0));
    }
}
