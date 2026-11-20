//! Stage 1: distance-driven selection with hysteresis.
//!
//! A node of level `L` is worth refining (expanding its children, or for a
//! level-1 node uploading its brick) when the cells refining would reveal,
//! `4^(L-1)` voxels across, cover at least `detail_pixels` pixels at the
//! node's distance. A refined node stays refined until those cells shrink
//! below `detail_pixels * hysteresis`, so a camera hovering at a threshold
//! does not make nodes flicker in and out. Nodes of level
//! [`ALWAYS_EXPANDED_LEVEL`] and up (and the root) are always expanded.
//!
//! A whole walk visits every resident record, one table lookup each, which
//! is too much for one frame once the table holds hundreds of thousands of
//! records. [`select_budgeted`] splits it in two:
//!
//! - **Growth** (expand and load) runs whole every frame but only descends
//!   into nodes that want refining. A child's cells are a quarter of its
//!   parent's and never closer, so it projects to less than a quarter of
//!   its parent's pixels: below a node that does not want refining nothing
//!   does. The walk is bounded by what is near the camera, not by the world.
//! - **Shrink** (collapse and evict) must look at every record, so it visits
//!   at most a set number per frame and resumes where it stopped.
//!
//! Growth is a function of the view and the table alone, so the frame loop
//! skips it while the camera stays within `regrow_voxels` of where it last
//! ran and nothing in the table changed since (see [`GrowthState`]).
//!
//! A shrink pass that fits the budget gives exactly [`select`]'s answer.

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

/// The squared distance, per level, within which a node wants refining:
/// [`projected_pixels`] `>= detail` exactly when `max(d, 1) <= cell /
/// (pixel_angle * detail)`, so when that radius `r` is at least one voxel,
/// when `d * d <= r * r`, and never otherwise.
struct Reach([f64; 16]);

impl Reach {
    fn new(view: &ViewPoint, detail: f32) -> Self {
        let angle = f64::from(view.pixel_angle.max(1e-6));
        Self(std::array::from_fn(|level| {
            if level == 0 {
                return -1.0;
            }
            let cell = (1u64 << (2 * (level - 1))) as f64;
            let r = cell / (angle * f64::from(detail));
            if r >= 1.0 { r * r } else { -1.0 }
        }))
    }

    fn wants(&self, level: u8, d2: f64) -> bool {
        let r2 = self.0.get(level as usize).copied().unwrap_or(f64::MAX);
        r2 >= 0.0 && d2 <= r2
    }
}

/// Distance along one axis from `e` to `[min, max]` (0 inside).
fn axis_distance(e: f64, min: f64, max: f64) -> f64 {
    if e < min {
        min - e
    } else if e > max {
        e - max
    } else {
        0.0
    }
}

/// Squared distance from `eye` to the box of `key`, in `f64`.
fn distance2(origin: [f64; 3], key: NodeKey, eye: [f64; 3]) -> f64 {
    let lo = key.min_local();
    let span = key.span() as f64;
    (0..3)
        .map(|a| {
            let min = origin[a] + f64::from(lo[a]);
            let d = axis_distance(eye[a], min, min + span);
            d * d
        })
        .sum()
}

/// When growth last ran, to skip it while its answer cannot have changed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrowthState {
    /// The view growth last ran for.
    last: Option<ViewPoint>,
    /// The table changed since.
    changed: bool,
}

impl GrowthState {
    /// Whether growth must run for `view`: never ran, the table changed,
    /// the pixel angle changed or the eye moved more than `regrow_voxels`.
    pub fn due(&self, view: &ViewPoint, regrow_voxels: f32) -> bool {
        let Some(last) = self.last else {
            return true;
        };
        let moved2: f32 = (0..3).map(|a| (view.eye[a] - last.eye[a]).powi(2)).sum();
        self.changed
            || last.pixel_angle != view.pixel_angle
            || moved2 > regrow_voxels * regrow_voxels
    }

    /// Growth ran for `view`.
    pub fn ran(&mut self, view: ViewPoint) {
        self.last = Some(view);
        self.changed = false;
    }

    /// The table changed: the next frame runs growth.
    pub fn changed(&mut self) {
        self.changed = true;
    }
}

/// Walks the whole resident tree and lists what to change.
pub fn select(res: &Residency, view: &ViewPoint, lod: &LodTuning) -> Selection {
    let mut walk = Vec::new();
    select_budgeted(res, &mut walk, view, lod, u32::MAX, true)
}

/// Lists what to change: when `grow`, every node that wants growing (a walk
/// bounded by the view), and what shrinking the walk in `walk` finds in at
/// most `max_visits` records (a new walk from the root when it is empty).
/// Records removed since they were queued are skipped; children of nodes
/// expanded since are visited by the next pass.
pub fn select_budgeted(
    res: &Residency,
    walk: &mut Vec<NodeKey>,
    view: &ViewPoint,
    lod: &LodTuning,
    max_visits: u32,
    grow: bool,
) -> Selection {
    let shape = res.shape();
    let always = ALWAYS_EXPANDED_LEVEL.min(shape.root_level());
    let mut out = Selection::default();
    let mut expand: Vec<(f32, NodeKey)> = Vec::new();
    let mut load: Vec<(f32, NodeKey)> = Vec::new();
    // Pushes the children of an expanded node, highest slot first, so
    // slots pop in ascending order.
    let push_children = |stack: &mut Vec<NodeKey>, key: NodeKey, mut mask: u64| {
        while mask != 0 {
            let slot = 63 - mask.leading_zeros();
            mask &= !(1u64 << slot);
            stack.push(key.child(slot));
        }
    };

    // Growth: only where refining is wanted (or forced). Children are
    // judged from their squared distances before any table lookup, so only
    // nodes that want refining cost one.
    let reach = Reach::new(view, lod.detail_pixels);
    let eye = view.eye.map(f64::from);
    let origin = shape.origin.to_array().map(f64::from);
    let forced = |level: u8| level >= 2 && u32::from(level) >= always;
    let mut stack = Vec::new();
    let root = shape.root();
    if grow && (forced(root.level) || reach.wants(root.level, distance2(origin, root, eye))) {
        stack.push(root);
    }
    while let Some(key) = stack.pop() {
        let Some(node) = res.node_state(key) else {
            continue;
        };
        if key.level >= 2 {
            if node.expanded {
                // Per-axis squared distances to the four child slabs.
                let lo = key.min_local();
                let child_span = (key.span() / 4) as f64;
                let slabs: [[f64; 4]; 3] = std::array::from_fn(|a| {
                    std::array::from_fn(|i| {
                        let min = origin[a] + f64::from(lo[a]) + child_span * i as f64;
                        let d = axis_distance(eye[a], min, min + child_span);
                        d * d
                    })
                });
                let child_level = key.level - 1;
                let mut mask = node.mask;
                while mask != 0 {
                    let slot = 63 - mask.leading_zeros();
                    mask &= !(1u64 << slot);
                    let child = key.child(slot);
                    let (ox, oy, oz) = (
                        (child.x - key.x * 4) as usize,
                        (child.y - key.y * 4) as usize,
                        (child.z - key.z * 4) as usize,
                    );
                    let d2 = slabs[0][ox] + slabs[1][oy] + slabs[2][oz];
                    if forced(child_level) || reach.wants(child_level, d2) {
                        stack.push(child);
                    }
                }
            } else {
                let priority = if forced(key.level) {
                    f32::MAX
                } else {
                    projected_pixels(shape.origin, key, view)
                };
                expand.push((priority, key));
            }
        } else if !node.brick_resident {
            load.push((projected_pixels(shape.origin, key, view), key));
        }
    }

    // Shrink: everything, resumably.
    if walk.is_empty() {
        walk.push(shape.root());
    }
    let mut visits = 0u32;
    while visits < max_visits {
        let Some(key) = walk.pop() else {
            break;
        };
        let Some(node) = res.node_state(key) else {
            continue;
        };
        visits += 1;
        let px = projected_pixels(shape.origin, key, view);
        let keep = px >= lod.detail_pixels * lod.hysteresis;
        if key.level >= 2 {
            let forced = key.level() >= always;
            if node.expanded {
                if forced || keep {
                    push_children(walk, key, node.mask);
                } else {
                    out.collapse.push(key);
                }
            }
        } else if node.brick_resident && !keep {
            out.evict.push(key);
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
    fn growth_reruns_after_moves_and_changes_only() {
        let view = ViewPoint {
            eye: [0.0, 0.0, 0.0],
            pixel_angle: 0.01,
        };
        let mut g = GrowthState::default();
        assert!(g.due(&view, 16.0), "never ran");
        g.ran(view);
        assert!(!g.due(&view, 16.0));
        let near = ViewPoint {
            eye: [10.0, 0.0, 5.0],
            ..view
        };
        assert!(!g.due(&near, 16.0), "moved less than 16 voxels");
        assert!(g.due(
            &ViewPoint {
                eye: [17.0, 0.0, 0.0],
                ..view
            },
            16.0
        ));
        assert!(g.due(
            &ViewPoint {
                pixel_angle: 0.02,
                ..view
            },
            16.0
        ));
        assert!(g.due(&near, 0.0), "0 runs every frame once moved");
        g.changed();
        assert!(g.due(&view, 16.0), "the table changed");
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
