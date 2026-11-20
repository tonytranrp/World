//! Which chunks the player needs: full-detail chunks in a sphere, coarse
//! chunk columns in a disc, both nearest first, and while the player moves,
//! the chunks ahead first and a lookahead sphere around where they will be.

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

/// The full-detail `sphere` around `eye` in the order to load it while the
/// player heads for `predicted`: by distance to the nearer of the two, so
/// chunks ahead come before chunks behind at the same distance and chunks
/// right around the player still come early. Ties in coordinate order.
pub fn ahead_first(eye: VoxelPos, predicted: VoxelPos, sphere: &[ChunkCoord]) -> Vec<ChunkCoord> {
    let mut keyed: Vec<(u32, ChunkCoord)> = sphere
        .iter()
        .map(|&c| {
            let d = chunk_distance(eye, c).min(chunk_distance(predicted, c));
            (d.to_bits(), c)
        })
        .collect();
    keyed.sort_unstable();
    keyed.into_iter().map(|(_, c)| c).collect()
}

/// The chunks within `radius_m` of `predicted` that lie outside the
/// full-detail radius `full_m` of `eye` but within `keep_m` of it (beyond
/// that they would be evicted at once), nearest to `predicted` first.
pub fn lookahead(
    eye: VoxelPos,
    predicted: VoxelPos,
    radius_m: f32,
    full_m: f32,
    keep_m: f32,
) -> Vec<ChunkCoord> {
    let (full, keep) = (voxels(full_m), voxels(keep_m));
    sphere(predicted, radius_m)
        .into_iter()
        .filter(|&c| {
            let d = chunk_distance(eye, c);
            d > full && d <= keep
        })
        .collect()
}

/// The player's velocity, estimated from successive eye positions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    last: Option<VoxelPos>,
    /// Voxels per second, smoothed.
    velocity: [f32; 3],
}

impl Motion {
    /// Takes the eye position `dt_secs` after the previous one. The estimate
    /// follows the measured velocity with a time constant of about
    /// `smoothing_secs` (0 follows it at once), in a rational form with no
    /// platform math functions, so every machine computes the same values.
    /// A step longer than `jump` voxels is a teleport, not motion, and
    /// resets the estimate.
    pub fn observe(&mut self, eye: VoxelPos, dt_secs: f32, smoothing_secs: f32, jump: f32) {
        let Some(last) = self.last.replace(eye) else {
            return;
        };
        if dt_secs.is_nan() || dt_secs <= 0.0 {
            return;
        }
        let step = [eye.x - last.x, eye.y - last.y, eye.z - last.z].map(|d| d as f32);
        if length(step) > jump {
            self.velocity = [0.0; 3];
            return;
        }
        let alpha = dt_secs / (smoothing_secs.max(0.0) + dt_secs);
        for (v, s) in self.velocity.iter_mut().zip(step) {
            *v += alpha * (s / dt_secs - *v);
        }
    }

    /// The smoothed velocity, in voxels per second.
    pub fn velocity(&self) -> [f32; 3] {
        self.velocity
    }

    /// Where the player will be after `secs` at the current velocity, the
    /// offset shortened to at most `max_offset` voxels. `None` when the
    /// offset is under half a chunk (standing still or nearly) or the point
    /// is outside the world.
    pub fn predicted(&self, eye: VoxelPos, secs: f32, max_offset: f32) -> Option<VoxelPos> {
        let mut offset = self.velocity.map(|v| v * secs);
        let len = length(offset);
        if len.is_nan() || len < (CHUNK_SIZE / 2) as f32 {
            return None;
        }
        if len > max_offset {
            offset = offset.map(|o| o * (max_offset / len));
        }
        let at = |e: i32, o: f32| (e as f32 + o).round() as i32;
        let p = VoxelPos::new(
            at(eye.x, offset[0]),
            at(eye.y, offset[1]),
            at(eye.z, offset[2]),
        );
        p.in_world().then_some(p)
    }

    /// Whether `chunk`'s centre lies ahead of `eye` along the motion. Nothing
    /// is ahead while standing still.
    pub fn is_ahead(&self, eye: VoxelPos, chunk: ChunkCoord) -> bool {
        let min = chunk.min_voxel();
        let half = (CHUNK_SIZE / 2) as f32;
        let d = [
            min.x as f32 + half - eye.x as f32,
            min.y as f32 + half - eye.y as f32,
            min.z as f32 + half - eye.z as f32,
        ];
        let v = self.velocity;
        d[0] * v[0] + d[1] * v[1] + d[2] * v[2] > 0.0
    }
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_follows_steady_walking_and_resets_on_a_teleport() {
        let mut motion = Motion::default();
        let dt = 1.0 / 60.0;
        // 4 m/s along +x is 256 voxels per second; steps alternate 4 and 5
        // voxels as a real eye's integer position does.
        let mut x = 0;
        for i in 0..600 {
            x += if i % 2 == 0 { 4 } else { 5 };
            motion.observe(VoxelPos::new(x, 100, 0), dt, 0.25, 1000.0);
        }
        let v = motion.velocity();
        assert!((v[0] - 270.0).abs() < 10.0, "{v:?}");
        assert!(v[1].abs() < 1e-3 && v[2].abs() < 1e-3, "{v:?}");
        let eye = VoxelPos::new(x, 100, 0);
        let ahead = ChunkCoord::containing(VoxelPos::new(x + 2048, 100, 0)).unwrap();
        let behind = ChunkCoord::containing(VoxelPos::new(x - 2048, 100, 0)).unwrap();
        assert!(motion.is_ahead(eye, ahead));
        assert!(!motion.is_ahead(eye, behind));

        motion.observe(VoxelPos::new(x + 5000, 100, 0), dt, 0.25, 1000.0);
        assert_eq!(motion.velocity(), [0.0; 3], "a jump is not motion");
        assert!(!motion.is_ahead(eye, ahead));
    }

    /// A motion estimate of `v` voxels per second.
    fn moving(v: [f32; 3]) -> Motion {
        let mut m = Motion::default();
        m.observe(VoxelPos::new(0, 0, 0), 1.0, 0.0, 1e9);
        m.observe(
            VoxelPos::new(v[0] as i32, v[1] as i32, v[2] as i32),
            1.0,
            0.0,
            1e9,
        );
        m
    }

    #[test]
    fn the_prediction_follows_the_velocity_within_limits() {
        let eye = VoxelPos::new(1000, 200, -3000);
        let m = moving([768.0, 0.0, 0.0]);
        assert_eq!(
            m.predicted(eye, 2.0, 1e9),
            Some(VoxelPos::new(2536, 200, -3000))
        );
        assert_eq!(
            m.predicted(eye, 2.0, 1000.0),
            Some(VoxelPos::new(2000, 200, -3000)),
            "shortened"
        );
        assert_eq!(m.predicted(eye, 0.0, 1e9), None, "no lookahead");
        assert_eq!(
            moving([100.0, 0.0, 0.0]).predicted(eye, 1.0, 1e9),
            None,
            "slow"
        );
    }

    #[test]
    fn chunks_ahead_load_first_and_the_sphere_comes_before_the_lookahead() {
        let eye = VoxelPos::new(512, 300, 512);
        // 12 m/s along +x for 2 s.
        let predicted = moving([768.0, 0.0, 0.0])
            .predicted(eye, 2.0, voxels(96.0))
            .unwrap();
        let full = sphere(eye, 64.0);
        let order = ahead_first(eye, predicted, &full);
        // The same chunks, each once.
        let mut a = order.clone();
        let mut b = full.clone();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
        // Most of the first quarter lies ahead (chunks right behind the
        // player keep their place); nearest first alone would put as many
        // behind as ahead.
        let quarter = &order[..order.len() / 4];
        let eye_chunk = ChunkCoord::containing(eye).unwrap();
        let ahead = quarter.iter().filter(|c| c.x() > eye_chunk.x()).count();
        let behind = quarter.iter().filter(|c| c.x() < eye_chunk.x()).count();
        assert!(ahead > 2 * behind, "{ahead} ahead, {behind} behind");
        let plain_behind = full[..full.len() / 4]
            .iter()
            .filter(|c| c.x() < eye_chunk.x())
            .count();
        assert!(plain_behind > behind);
        // The player's own chunk and its neighbours still come early.
        let own = order.iter().position(|&c| c == eye_chunk).unwrap();
        assert!(own < order.len() / 8, "{own}");

        // The lookahead adds only chunks outside the sphere, kept by the
        // evict radius, all ahead.
        let extra = lookahead(eye, predicted, 64.0, 64.0, 96.0);
        assert!(!extra.is_empty());
        for c in &extra {
            assert!(!full.contains(c));
            let d = chunk_distance(eye, *c);
            assert!(d > voxels(64.0) && d <= voxels(96.0));
            assert!(c.x() > eye_chunk.x(), "{c:?}");
        }
        // Deterministic: the same inputs give the same order.
        assert_eq!(order, ahead_first(eye, predicted, &full));
        assert_eq!(extra, lookahead(eye, predicted, 64.0, 64.0, 96.0));
    }

    #[test]
    fn standing_still_decays_the_estimate() {
        let mut motion = Motion::default();
        let dt = 0.1;
        motion.observe(VoxelPos::new(0, 0, 0), dt, 0.0, 1e9);
        motion.observe(VoxelPos::new(100, 0, 0), dt, 0.0, 1e9);
        assert_eq!(motion.velocity(), [1000.0, 0.0, 0.0], "no smoothing");
        for _ in 0..100 {
            motion.observe(VoxelPos::new(100, 0, 0), dt, 0.5, 1e9);
        }
        assert!(motion.velocity()[0].abs() < 1e-3, "{:?}", motion.velocity());
    }

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
