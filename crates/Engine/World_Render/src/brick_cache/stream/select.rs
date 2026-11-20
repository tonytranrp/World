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
//! records. [`select_split`] splits it in two, each with its own budget per
//! frame (growth `lod.max_visits_per_frame`, shrink
//! `lod.max_shrink_visits_per_frame` and `lod.max_collapses_per_frame`),
//! and both resume where they stopped:
//!
//! - **Growth** (expand and load) only descends into nodes that want
//!   refining. A child's cells are a quarter of its parent's and never
//!   closer, so it projects to less than a quarter of its parent's pixels:
//!   below a node that does not want refining nothing does. A pass is
//!   best-first: the nodes covering the most pixels are visited first, so a
//!   pass the budget splits finds the nearest work in its first frame.
//!   Candidates the upload budget turns away are carried to the next frame.
//! - **Shrink** (collapse and evict) must look at every expanded node and
//!   resident brick, so it is a plain resumable walk; it skips children
//!   without records and level-1 children without bricks.
//!
//! Both are functions of the view, the pressure and the table alone, so a
//! new pass of either starts only once the camera moved more than
//! `regrow_voxels` from where that half's last pass started, the pixel
//! angle changed, or the table changed since (see [`GrowthState`]). A still
//! camera over a settled cache walks nothing.
//! Nodes expanded while a pass runs rejoin it, so their children are judged
//! by the same pass. A growth pass whose camera moved more than
//! `restart_voxels` (at least `regrow_voxels`) starts over from the root, so
//! a moving camera works on its current view, nearest and coarsest first;
//! smaller moves let it finish, since every node is checked against the
//! current view when it is taken. Every node taken off a queue or put on
//! one counts against the budget, whether or not it still exists. Only
//! children with records of their own are queued, and of a level-2 node's
//! only those whose bricks are not resident, so a settled region costs one
//! visit per expanded node instead of one per child.
//!
//! Nodes in a replaced chunk that waits to be applied are skipped: it is
//! rebuilt when applied.
//!
//! A growth pass or a shrink pass run to completion under one view gives
//! exactly [`select`]'s answer.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

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
    let o = origin.to_array().map(f64::from);
    pixels_from_d2(key, distance2(o, key, view.eye.map(f64::from)), view)
}

/// [`projected_pixels`] from the node's squared distance (as
/// `distance2` computes it), so a walk that already has it gets the same
/// value bit for bit.
fn pixels_from_d2(key: NodeKey, d2: f64, view: &ViewPoint) -> f32 {
    let cell = (1u64 << (2 * (key.level() - 1))) as f32;
    let d = (d2.sqrt() as f32).max(1.0);
    cell / (d * view.pixel_angle.max(1e-6))
}

/// The squared distance, per level, within which a node wants refining:
/// [`projected_pixels`] `>= detail` exactly when `max(d, 1) <= cell /
/// (pixel_angle * detail)`, so when that radius `r` is at least one voxel,
/// when `d * d <= r * r`, and never otherwise.
struct Reach([f64; 16]);

impl Reach {
    /// `pressure` multiplies the threshold of levels 2 and up only: the
    /// nodes whose children take node pool records.
    fn new(view: &ViewPoint, detail: f32, pressure: f32) -> Self {
        let angle = f64::from(view.pixel_angle.max(1e-6));
        Self(std::array::from_fn(|level| {
            if level == 0 {
                return -1.0;
            }
            let scale = if level >= 2 { pressure } else { 1.0 };
            let cell = (1u64 << (2 * (level - 1))) as f64;
            let r = cell / (angle * f64::from(detail * scale));
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

/// A node waiting in a growth pass, ordered by projected pixels (ties to the
/// lower key, so passes are deterministic).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    pixels: f32,
    key: NodeKey,
}

impl Eq for Pending {}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        self.pixels
            .total_cmp(&other.pixels)
            .then_with(|| other.key.cmp(&self.key))
    }
}

impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Stage 1 across frames: the growth pass in progress, when it and the last
/// shrink pass started, and the candidates the upload budget has not taken
/// yet.
#[derive(Clone, Debug, Default)]
pub struct GrowthState {
    /// The view the last shrink pass started with.
    shrink_last: Option<ViewPoint>,
    /// The table changed since the last shrink pass started.
    shrink_changed: bool,
    /// The view the last pass started with.
    last: Option<ViewPoint>,
    /// The table changed since the last pass started.
    changed: bool,
    /// Nodes the pass in progress still has to visit.
    frontier: BinaryHeap<Pending>,
    /// A pass is in progress.
    active: bool,
    /// Candidates found but not taken yet; checked again before use.
    carry_expand: Vec<NodeKey>,
    carry_load: Vec<NodeKey>,
    /// Pops and pushes the last growth step made.
    visits: u32,
    /// Records the last shrink step took.
    shrink_visits: u32,
}

/// Whether a pass that last started at `last` must start again for `view`.
fn pass_due(last: Option<ViewPoint>, changed: bool, view: &ViewPoint, regrow_voxels: f32) -> bool {
    let Some(last) = last else {
        return true;
    };
    let moved2: f32 = (0..3).map(|a| (view.eye[a] - last.eye[a]).powi(2)).sum();
    changed || last.pixel_angle != view.pixel_angle || moved2 > regrow_voxels * regrow_voxels
}

impl GrowthState {
    /// Whether a new pass must start for `view`: none ran yet, the table
    /// changed, the pixel angle changed or the eye moved more than
    /// `regrow_voxels` since the last pass started.
    pub fn due(&self, view: &ViewPoint, regrow_voxels: f32) -> bool {
        pass_due(self.last, self.changed, view, regrow_voxels)
    }

    /// Whether a new shrink pass must start for `view`, by the same rule.
    pub fn shrink_due(&self, view: &ViewPoint, regrow_voxels: f32) -> bool {
        pass_due(self.shrink_last, self.shrink_changed, view, regrow_voxels)
    }

    /// Starts a shrink pass for `view`.
    pub fn shrink_ran(&mut self, view: ViewPoint) {
        self.shrink_last = Some(view);
        self.shrink_changed = false;
    }

    /// Something outside the table makes shrinking worth another look (stage
    /// 2's request holds expire with time).
    pub fn shrink_changed(&mut self) {
        self.shrink_changed = true;
    }

    /// Starts a pass for `view`, dropping the rest of any pass in progress.
    pub fn ran(&mut self, view: ViewPoint) {
        self.last = Some(view);
        self.changed = false;
        self.frontier.clear();
        self.active = true;
    }

    /// The table changed: a new pass of each half starts once its current
    /// one is done.
    pub fn changed(&mut self) {
        self.changed = true;
        self.shrink_changed = true;
    }

    /// A pass is in progress.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The view the pass in progress (or the last one) started with.
    pub fn started_at(&self) -> Option<ViewPoint> {
        self.last
    }

    /// Queue pops and pushes the last growth step made, which the visit
    /// budget bounds (one node's pushes may overshoot it by up to 64).
    pub fn last_visits(&self) -> u32 {
        self.visits
    }

    /// Records the last shrink step took off its walk.
    pub fn last_shrink_visits(&self) -> u32 {
        self.shrink_visits
    }

    /// Node `key`, just expanded, rejoins the pass in progress so its
    /// children are judged. Without a pass in progress the next pass will.
    pub fn rejoin(&mut self, key: NodeKey, pixels: f32) {
        if self.active {
            self.frontier.push(Pending { pixels, key });
        } else {
            self.changed = true;
        }
    }

    /// Candidates the frame did not get to, for the next frame.
    pub fn carry(&mut self, expand: &[NodeKey], load: &[NodeKey]) {
        self.carry_expand.clear();
        self.carry_expand.extend_from_slice(expand);
        self.carry_load.clear();
        self.carry_load.extend_from_slice(load);
    }
}

/// Walks the whole resident tree and lists what to change.
pub fn select(res: &Residency, view: &ViewPoint, lod: &LodTuning) -> Selection {
    let mut walk = Vec::new();
    let mut growth = GrowthState::default();
    select_budgeted(res, &mut walk, &mut growth, view, lod, 1.0, u32::MAX)
}

/// Lists what to change. Growth starts a pass when one is due and none is
/// in progress, then visits at most `max_visits` nodes of the pass in
/// progress; the candidates it finds join those carried from earlier
/// frames that still apply. Shrink visits at most `max_visits` records of
/// the walk in `walk`, which starts again from the root when it is empty
/// and a shrink pass is due.
/// `pressure` (at least 1) raises the detail threshold of levels 2 and up,
/// whose expansions take node pool records; bricks keep theirs. Records
/// removed since they were queued are skipped.
pub fn select_budgeted(
    res: &Residency,
    walk: &mut Vec<NodeKey>,
    growth: &mut GrowthState,
    view: &ViewPoint,
    lod: &LodTuning,
    pressure: f32,
    max_visits: u32,
) -> Selection {
    select_split(
        res,
        walk,
        growth,
        view,
        lod,
        pressure,
        Budgets::visits(max_visits),
    )
}

/// What one call of [`select_split`] may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    /// Growth queue pops and pushes.
    pub growth_visits: u32,
    /// Records the shrink walk takes.
    pub shrink_visits: u32,
    /// Collapses the shrink walk may find; it stops at the last one and
    /// resumes there, since releasing a subtree costs per record.
    pub collapses: u32,
}

impl Budgets {
    /// Both walks at `visits`, collapses unbounded.
    pub fn visits(visits: u32) -> Self {
        Self {
            growth_visits: visits,
            shrink_visits: visits,
            collapses: u32::MAX,
        }
    }
}

/// [`select_budgeted`] with separate budgets for growth and shrink.
pub fn select_split(
    res: &Residency,
    walk: &mut Vec<NodeKey>,
    growth: &mut GrowthState,
    view: &ViewPoint,
    lod: &LodTuning,
    pressure: f32,
    budgets: Budgets,
) -> Selection {
    let max_visits = budgets.growth_visits;
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
    let reach = Reach::new(view, lod.detail_pixels, pressure);
    let eye = view.eye.map(f64::from);
    let origin = shape.origin.to_array().map(f64::from);
    let forced = |level: u8| level >= 2 && u32::from(level) >= always;
    // Whether `key` wants refining from here, given its squared distance.
    let wanted = |key: NodeKey, d2: f64| forced(key.level) || reach.wants(key.level, d2);
    // Pixels a node's refined cells cover, from its squared distance: the
    // order of the pass. Forced nodes come first.
    let pixels = |key: NodeKey, d2: f64| {
        if forced(key.level) {
            f32::MAX
        } else {
            pixels_from_d2(key, d2, view)
        }
    };
    let root = shape.root();
    // A pass whose camera moved on restarts: its queue was ordered for a
    // view that is gone. Table changes wait for the pass to finish (nodes
    // expanded meanwhile rejoin it).
    let restart = lod.restart_voxels.max(lod.regrow_voxels);
    let moved = growth.active && pass_due(growth.last, false, view, restart);
    if moved || (!growth.active && growth.due(view, lod.regrow_voxels)) {
        growth.ran(*view);
        let d2 = distance2(origin, root, eye);
        if wanted(root, d2) {
            growth.frontier.push(Pending {
                pixels: pixels(root, d2),
                key: root,
            });
        }
    }
    let mut visits = 0u32;
    while visits < max_visits {
        let Some(Pending { key, .. }) = growth.frontier.pop() else {
            growth.active = false;
            break;
        };
        // Every pop counts, so a queue gone stale still costs a bounded
        // amount per frame.
        visits += 1;
        // The eye may have moved since the node was queued.
        if !wanted(key, distance2(origin, key, eye)) {
            continue;
        }
        let Some(node) = res.node_state(key) else {
            continue;
        };
        // A replaced chunk not applied yet is rebuilt when it is: nothing
        // in it is worth refining until then.
        if res.is_blocked(key) {
            continue;
        }
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
                // Only children with records can need work; of a level-2
                // node's, only those whose bricks are not resident.
                let mut mask = if key.level == 2 {
                    node.kids & !node.bricks
                } else {
                    node.kids
                };
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
                    if wanted(child, d2) {
                        // Pushes count too: a node can queue 64 children.
                        visits += 1;
                        growth.frontier.push(Pending {
                            pixels: pixels(child, d2),
                            key: child,
                        });
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
    if growth.frontier.is_empty() {
        growth.active = false;
    }
    growth.visits = visits;

    // Candidates carried from earlier frames, if they still apply.
    for key in std::mem::take(&mut growth.carry_expand) {
        let still = res.node_state(key).is_some_and(|n| !n.expanded)
            && !res.is_blocked(key)
            && wanted(key, distance2(origin, key, eye));
        if still {
            let priority = if forced(key.level) {
                f32::MAX
            } else {
                projected_pixels(shape.origin, key, view)
            };
            expand.push((priority, key));
        }
    }
    for key in std::mem::take(&mut growth.carry_load) {
        let still = res.node_state(key).is_some_and(|n| !n.brick_resident)
            && !res.is_blocked(key)
            && wanted(key, distance2(origin, key, eye));
        if still {
            load.push((projected_pixels(shape.origin, key, view), key));
        }
    }
    for list in [&mut expand, &mut load] {
        list.sort_unstable_by_key(|&(_, k)| k);
        list.dedup_by_key(|&mut (_, k)| k);
    }

    // Shrink: everything, resumably.
    if walk.is_empty() && growth.shrink_due(view, lod.regrow_voxels) {
        growth.shrink_ran(*view);
        walk.push(shape.root());
    }
    let mut visits = 0u32;
    while visits < budgets.shrink_visits && out.collapse.len() < budgets.collapses as usize {
        let Some(key) = walk.pop() else {
            break;
        };
        visits += 1;
        let Some(node) = res.node_state(key) else {
            continue;
        };
        let px = projected_pixels(shape.origin, key, view);
        let scale = if key.level >= 2 { pressure } else { 1.0 };
        let keep = px >= lod.detail_pixels * scale * lod.hysteresis;
        if key.level >= 2 {
            let forced = key.level() >= always;
            if node.expanded {
                if forced || keep {
                    // Only children with records can shrink; of a level-2
                    // node's, only those with resident bricks.
                    let kids = if key.level == 2 {
                        node.bricks
                    } else {
                        node.kids
                    };
                    push_children(walk, key, kids);
                } else {
                    out.collapse.push(key);
                }
            }
        } else if node.brick_resident && !keep {
            out.evict.push(key);
        }
    }
    growth.shrink_visits = visits;
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
    fn shrink_reruns_after_moves_and_changes_only() {
        let view = ViewPoint {
            eye: [0.0, 0.0, 0.0],
            pixel_angle: 0.01,
        };
        let mut g = GrowthState::default();
        assert!(g.shrink_due(&view, 16.0), "never ran");
        g.shrink_ran(view);
        assert!(!g.shrink_due(&view, 16.0));
        g.ran(view);
        assert!(
            !g.shrink_due(&view, 16.0),
            "a growth pass is not a shrink pass"
        );
        let far = ViewPoint {
            eye: [0.0, 17.0, 0.0],
            ..view
        };
        assert!(g.shrink_due(&far, 16.0));
        g.changed();
        assert!(g.shrink_due(&view, 16.0), "the table changed");
        g.shrink_ran(view);
        g.shrink_changed();
        assert!(g.shrink_due(&view, 16.0), "asked to look again");
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
