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
use super::select::{ViewPoint, distance, select};
use crate::brick_cache::config::{BrickCacheConfig, Stage};
use crate::brick_cache::evict::eviction_target;
use crate::brick_cache::feedback::{Request, Victim};

/// Frames a node stays expanded in stage 2 after a ray last asked for it,
/// whatever its distance.
pub const REQUEST_HOLD_FRAMES: u32 = 60;

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
    /// Bytes of the serialized upload.
    pub upload_bytes: u64,
    /// Victims to ask the eviction pass for, if it should run.
    pub evict_target: Option<u32>,
}

/// Runs one frame of streaming into `batch` (cleared first).
pub fn run_frame<S: VoxelSource>(
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

    // Stage work, decided before edits so their reserve is known.
    let selection = input
        .view
        .map(|view| select(res, &view, &input.config.lod))
        .unwrap_or_default();
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

    // Growth, by priority, while the budget lasts.
    for k in expands {
        if cost(batch) >= budget {
            break;
        }
        match res.expand(source, k, batch) {
            Ok(_) => report.expanded += 1,
            Err(Refused::PoolFull) => report.refused_full += 1,
            Err(_) => {}
        }
    }
    let mut node_full = false;
    for k in loads {
        if cost(batch) >= budget || node_full {
            break;
        }
        match res.load_brick(source, k, batch) {
            Ok(_) => report.loaded += 1,
            Err(Refused::PoolFull) => {
                report.refused_full += 1;
                node_full = true;
            }
            Err(_) => {}
        }
    }

    report.upload_bytes = cost(batch);
    let stats = res.stats();
    report.evict_target = eviction_target(
        res.brick_fill(),
        input.config.evict_threshold,
        input.config.evict_fraction,
        stats.bricks,
        input.victim_capacity,
    )
    .or_else(|| {
        // A full pool below the threshold (fragmentation): free a little.
        (report.refused_full > 0 && stats.bricks > 0)
            .then_some((stats.bricks as u32 / 20).clamp(1, input.victim_capacity))
    });
    report
}
