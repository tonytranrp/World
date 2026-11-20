//! Stage 1: distance-driven selection with hysteresis.
//!
//! A node of level `L` is worth refining (expanding its children, or for a
//! level-1 node uploading its brick) when the cells refining would reveal,
//! `4^(L-1)` voxels across, cover at least `detail_pixels` pixels at the
//! node's distance. A refined node stays refined until those cells shrink
//! below `detail_pixels * hysteresis`, so a camera hovering at a threshold
//! does not make nodes flicker in and out. Nodes of level
//! [`ALWAYS_EXPANDED_LEVEL`] and up (and the root) are always expanded.

use world_core::VoxelPos;

use super::key::NodeKey;
use super::residency::Residency;
use crate::brick_cache::config::LodTuning;

/// The coarsest levels stay resident for the whole map (decision 0007).
pub const ALWAYS_EXPANDED_LEVEL: u32 = 6;

/// Where the camera is, in voxel units of the world frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewPoint {
    pub eye: [f32; 3],
    /// Radians one pixel spans vertically.
    pub pixel_angle: f32,
}

/// What stage 1 wants changed, most important first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub collapse: Vec<NodeKey>,
    pub evict: Vec<NodeKey>,
    /// Nodes to expand, by priority (projected pixels) descending.
    pub expand: Vec<NodeKey>,
    /// Bricks to load, by priority descending.
    pub load: Vec<NodeKey>,
}

/// Distance in voxels from `eye` to the box of `key` (0 inside).
pub fn distance(origin: VoxelPos, key: NodeKey, eye: [f32; 3]) -> f32 {
    let lo = key.min_local();
    let o = origin.to_array();
    let span = key.span() as f64;
    let mut d2 = 0.0f64;
    for a in 0..3 {
        let min = f64::from(o[a]) + f64::from(lo[a]);
        let max = min + span;
        let e = f64::from(eye[a]);
        let d = if e < min {
            min - e
        } else if e > max {
            e - max
        } else {
            0.0
        };
        d2 += d * d;
    }
    d2.sqrt() as f32
}

/// Pixels covered by the cells refining `key` would reveal.
pub fn projected_pixels(origin: VoxelPos, key: NodeKey, view: &ViewPoint) -> f32 {
    let cell = (1u64 << (2 * (key.level() - 1))) as f32;
    let d = distance(origin, key, view.eye).max(1.0);
    cell / (d * view.pixel_angle.max(1e-6))
}

/// Walks the resident tree and lists what to change.
pub fn select(res: &Residency, view: &ViewPoint, lod: &LodTuning) -> Selection {
    let shape = res.shape();
    let always = ALWAYS_EXPANDED_LEVEL.min(shape.root_level());
    let mut out = Selection::default();
    let mut expand: Vec<(f32, NodeKey)> = Vec::new();
    let mut load: Vec<(f32, NodeKey)> = Vec::new();
    let mut stack = vec![shape.root()];
    while let Some(key) = stack.pop() {
        if !res.has_record(key) {
            continue;
        }
        let px = projected_pixels(shape.origin, key, view);
        let want = px >= lod.detail_pixels;
        let keep = px >= lod.detail_pixels * lod.hysteresis;
        if key.level >= 2 {
            let forced = key.level() >= always;
            if res.is_expanded(key) {
                if forced || keep {
                    stack.extend(res.children_with_records(key));
                } else {
                    out.collapse.push(key);
                }
            } else if forced || want {
                expand.push((if forced { f32::MAX } else { px }, key));
            }
        } else if res.is_brick_resident(key) {
            if !keep {
                out.evict.push(key);
            }
        } else if want {
            load.push((px, key));
        }
    }
    let by_priority =
        |a: &(f32, NodeKey), b: &(f32, NodeKey)| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1));
    expand.sort_by(by_priority);
    load.sort_by(by_priority);
    out.collapse.sort_unstable();
    out.evict.sort_unstable();
    out.expand = expand.into_iter().map(|(_, k)| k).collect();
    out.load = load.into_iter().map(|(_, k)| k).collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_is_zero_inside_and_euclidean_outside() {
        let key = NodeKey {
            level: 1,
            x: 1,
            y: 0,
            z: 0,
        };
        let o = VoxelPos::new(0, 0, 0);
        assert_eq!(distance(o, key, [20.0, 5.0, 5.0]), 0.0);
        assert_eq!(distance(o, key, [40.0, 5.0, 5.0]), 8.0);
        assert_eq!(distance(o, key, [16.0, -3.0, -4.0]), 5.0);
    }

    #[test]
    fn refinement_falls_off_with_distance() {
        let o = VoxelPos::new(0, 0, 0);
        let key = NodeKey {
            level: 1,
            x: 0,
            y: 0,
            z: 0,
        };
        let near = ViewPoint {
            eye: [8.0, 8.0, -10.0],
            pixel_angle: 0.01,
        };
        let far = ViewPoint {
            eye: [8.0, 8.0, -1000.0],
            ..near
        };
        assert!(projected_pixels(o, key, &near) > projected_pixels(o, key, &far));
    }
}
