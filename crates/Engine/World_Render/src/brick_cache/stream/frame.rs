//! One frame of streaming: victims, edits, then stage 1 or stage 2 work,
//! under the byte budget.
//!
//! Budget order (decision 0007): changed resident bricks first. Requests and
//! stage-1 loads may claim at most three quarters of the budget, so edits
//! always get at least a quarter, plus whatever the loads leave. A changed
//! brick that does not fit is made not resident (rays then request it
//! again), never left stale. Record rewrites caused by edits and unlinks are
//! mandatory and always go out, whatever the budget.

use world_core::BrickPos;
use world_core::tree::VoxelSource;

use super::batch::UploadBatch;
use super::key::NodeKey;
use super::residency::{Refused, Residency};
use super::select::{ViewPoint, distance, projected_pixels, select_budgeted};
use crate::brick_cache::config::{BrickCacheConfig, Stage};
use crate::brick_cache::evict::eviction_target;
use crate::brick_cache::feedback::{Request, Victim};

/// Frames a node stays expanded in stage 2 after a ray last asked for it,
/// whatever its distance.
pub const REQUEST_HOLD_FRAMES: u32 = 60;

/// Node pool fill below which stage 1 asks for more detail again.
pub const PRESSURE_LOW: f32 = 0.80;
/// Frames after a rise before the pressure may fall, so a rise's collapses
/// settle instead of being undone at once.
pub const PRESSURE_COOLDOWN_FRAMES: u32 = 120;
/// The most stage 1 may raise its node detail threshold.
pub const PRESSURE_MAX: f32 = 64.0;

/// The next pressure multiplier: up 25% when the node pool refused an
/// expansion; down 5% when the pool is emptier than [`PRESSURE_LOW`] and no
/// rise happened in the last [`PRESSURE_COOLDOWN_FRAMES`]; unchanged
/// otherwise. Pressure raises the detail threshold of nodes (levels 2 and
/// up), so the shrink walk collapses the farthest expanded nodes, freeing
/// records for nearer ones, and stage 1 stops asking for expansions the pool
/// cannot hold. A full pool that refuses nothing is left alone.
pub fn adapt_pressure(pressure: f32, node_fill: f32, refused: bool, frames_since_rise: u32) -> f32 {
    if refused {
        (pressure * 1.25).min(PRESSURE_MAX)
    } else if node_fill < PRESSURE_LOW && frames_since_rise >= PRESSURE_COOLDOWN_FRAMES {
        (pressure / 1.05).max(1.0)
    } else {
        pressure
    }
}

/// What one frame gets to work with.
pub struct FrameInput<'a, S: VoxelSource> {
    pub source: &'a S,
    pub frame: u32,
    /// The world changed wholesale: drop everything and start over.
    pub refresh: bool,
    pub dirty: &'a [BrickPos],
    pub view: Option<ViewPoint>,
    pub requests: &'a [Request],
    pub victims: &'a [Victim],
    pub config: &'a BrickCacheConfig,
    pub budget: u64,
    /// Victims the eviction pass can return (the layout's capacity).
    pub victim_capacity: u32,
}

/// What one frame did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameReport {
    pub edited_uploaded: u32,
    pub edited_unlinked: u32,
    pub loaded: u32,
    pub expanded: u32,
    pub collapsed: u32,
    pub evicted: u32,
    pub victims_applied: u32,
    pub refused_full: u32,
    /// Expansions the node pool refused.
    pub refused_nodes: u32,
    /// Bytes of the serialized upload.
    pub upload_bytes: u64,
    /// Victims to ask the eviction pass for, if it should run.
    pub evict_target: Option<u32>,
}

/// Runs one frame of streaming into `batch` (cleared first).
pub fn run_frame<S: VoxelSource + Sync>(
    res: &mut Residency,
    input: &FrameInput<'_, S>,
    batch: &mut UploadBatch,
) -> FrameReport {
    batch.clear();
    let mut report = FrameReport::default();
    res.set_frame(input.frame);
    let source = input.source;
    let shape = res.shape();
    if input.refresh || !res.has_record(shape.root()) {
        res.reset(source, batch);
    }

    for v in input.victims {
        if res.evict_victim(v.id, v.block, batch) {
            report.victims_applied += 1;
        }
    }

    let cost = |batch: &UploadBatch| batch.serialized_words() as u64 * 4;
    let budget = input.budget.max(1);
    let mean = u64::from(input.config.mean_brick_bytes);

    // Stage work, decided before edits so their reserve is known. Under node
    // pool pressure stage 1 asks for coarser nodes.
    let lod = &input.config.lod;
    if input.config.stage == Stage::RayGuided {
        // Request holds expire with time, not with the view or the table.
        res.growth().shrink_changed();
    }
    let selection = match input.view {
        Some(view) => {
            let visits = if res.take_full_shrink() {
                u32::MAX
            } else {
                lod.max_visits_per_frame
            };
            let mut walk = res.take_walk();
            let mut growth = res.take_growth();
            let pressure = res.pressure();
            let s = select_budgeted(res, &mut walk, &mut growth, &view, lod, pressure, visits);
            res.put_walk(walk);
            res.put_growth(growth);
            s
        }
        None => Default::default(),
    };
    let mut loads: Vec<NodeKey> = Vec::new();
    let mut expands: Vec<NodeKey> = selection.expand.clone();
    match input.config.stage {
        Stage::Distance => loads.extend(&selection.load),
        Stage::RayGuided => {
            for r in input.requests {
                res.note_request(r.key);
                if r.key.level == 1 {
                    loads.push(r.key);
                } else if !expands.contains(&r.key) {
                    expands.push(r.key);
                }
            }
        }
    }
    let reserve = (loads.len() as u64 * mean).min(budget / 4 * 3);

    // Edits first.
    let mut dirty: Vec<NodeKey> = input
        .dirty
        .iter()
        .filter_map(|&b| shape.brick_key(b))
        .collect();
    dirty.sort_unstable();
    dirty.dedup();
    if let Some(view) = input.view {
        dirty.sort_by(|a, b| {
            distance(shape.origin, *a, view.eye)
                .total_cmp(&distance(shape.origin, *b, view.eye))
                .then(a.cmp(b))
        });
    }
    if !dirty.is_empty() {
        let edit_budget = budget - reserve;
        let mut spent = cost(batch);
        let (mut up, mut down) = (0, 0);
        res.apply_edits(
            source,
            &dirty,
            |bytes| {
                let fits = spent + bytes <= edit_budget;
                if fits {
                    spent += bytes;
                    up += 1;
                } else {
                    down += 1;
                }
                fits
            },
            batch,
        );
        report.edited_uploaded = up;
        report.edited_unlinked = down;
    }

    // Shrinking never needs budget.
    for &k in &selection.collapse {
        let hold = input.config.stage == Stage::RayGuided
            && res.last_requested(k) + REQUEST_HOLD_FRAMES > input.frame
            && res.last_requested(k) != 0;
        if !hold && res.collapse(k, batch).is_ok() {
            report.collapsed += 1;
        }
    }
    if input.config.stage == Stage::Distance {
        for &k in &selection.evict {
            if res.evict_brick(k, batch).is_ok() {
                report.evicted += 1;
            }
        }
    }

    // Growth, by priority, while the budget lasts. An expanded node rejoins
    // the growth pass so its children are judged; candidates the budget did
    // not reach are carried to the next frame.
    for &k in &expands {
        if cost(batch) >= budget {
            break;
        }
        match res.expand(source, k, batch) {
            Ok(_) => {
                report.expanded += 1;
                if let Some(view) = input.view {
                    let pixels = projected_pixels(shape.origin, k, &view);
                    res.growth().rejoin(k, pixels);
                }
            }
            Err(Refused::PoolFull) => {
                report.refused_full += 1;
                report.refused_nodes += 1;
            }
            Err(_) => {}
        }
    }
    // Encode about as many bricks as the budget left can take, in parallel.
    if cost(batch) < budget && !loads.is_empty() {
        let room = (budget - cost(batch)) / mean.max(1) + 1;
        let take = loads.len().min((room as usize).saturating_mul(5) / 4 + 8);
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().clamp(1, 8));
        let (loaded, full) =
            res.load_bricks(source, &loads[..take], threads, |b| cost(b) < budget, batch);
        report.loaded += loaded;
        if full {
            report.refused_full += 1;
        }
    }

    // Everything not done yet: the next frame checks each again and drops
    // what has since been expanded or loaded (or is no longer wanted).
    if input.config.stage == Stage::Distance {
        res.growth().carry(&expands, &loads);
    }

    report.upload_bytes = cost(batch);
    if input.config.stage == Stage::Distance {
        let before = res.pressure();
        let since = input.frame.wrapping_sub(res.pressure_raised_at());
        let after = adapt_pressure(before, res.node_fill(), report.refused_nodes > 0, since);
        if after != before {
            if after > before {
                res.set_pressure_raised_at(input.frame);
            }
            res.set_pressure(after);
            res.growth().changed();
        }
    }
    // Anything that changed the table can change what growth finds; edits
    // rewrite records even when no brick is uploaded.
    if !dirty.is_empty()
        || report.collapsed
            + report.evicted
            + report.victims_applied
            + report.edited_uploaded
            + report.edited_unlinked
            > 0
    {
        res.growth().changed();
    }
    let bricks = res.brick_count();
    report.evict_target = eviction_target(
        res.brick_fill(),
        input.config.evict_threshold,
        input.config.evict_fraction,
        bricks,
        input.victim_capacity,
    )
    .or_else(|| {
        // A full pool below the threshold (fragmentation): free a little.
        (report.refused_full > 0 && bricks > 0)
            .then_some((bricks as u32 / 20).clamp(1, input.victim_capacity))
    });
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_rises_on_refusals_and_falls_after_a_cooldown_when_drained() {
        let settled = PRESSURE_COOLDOWN_FRAMES;
        assert_eq!(adapt_pressure(1.0, 0.5, false, settled), 1.0);
        assert_eq!(
            adapt_pressure(1.0, 0.99, false, settled),
            1.0,
            "full, refusing nothing"
        );
        assert_eq!(
            adapt_pressure(2.0, 0.5, true, 0),
            2.5,
            "a refusal raises it"
        );
        assert_eq!(
            adapt_pressure(4.0, 0.9, false, settled),
            4.0,
            "held above the low mark"
        );
        assert_eq!(
            adapt_pressure(4.0, 0.5, false, 3),
            4.0,
            "held during the cooldown"
        );
        assert!(adapt_pressure(4.0, 0.5, false, settled) < 4.0);
        assert_eq!(
            adapt_pressure(1.01, 0.1, false, settled),
            1.0,
            "never below 1"
        );
        assert_eq!(adapt_pressure(PRESSURE_MAX, 1.0, true, 0), PRESSURE_MAX);
    }
}
