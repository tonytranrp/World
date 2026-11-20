//! The brick cache in Bevy's render world.
//!
//! Per frame:
//!
//! 1. Extract: the main world's [`VoxelWorldFeed`] is *taken* (each change
//!    exactly once) into [`PendingFeed`]; the config and the main camera's
//!    view point are copied.
//! 2. `RenderGraph` / `Begin` ([`run_brick_cache`]): finished read-backs are
//!    collected without waiting, one frame of streaming runs on the CPU, and
//!    the upload, the apply pass and (when the pool is full) the eviction
//!    pass are recorded ahead of every camera.
//! 3. Cameras trace and composite (`passes`).
//! 4. `RenderGraph` / `Render`, after the cameras ([`copy_feedback`]): the
//!    request block (and victims) go to a free read-back slot, and the
//!    request counter is cleared for the next frame.
//! 5. `RenderGraph` / `Finish` ([`map_feedback`]): the copied slots are mapped
//!    asynchronously.
//!
//! Nothing here ever blocks on the GPU.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_resource::{
    BindGroupLayoutDescriptor, CachedComputePipelineId, ComputePipelineDescriptor, PipelineCache,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::view::ExtractedView;
use wgpu::util::StagingBelt;
use world_core::ConfigAsset;
use world_core::{TreeSnapshot, VoxelTree};
use world_reload::{Config, ConfigOrigin, ConfigRegistry};

use super::config::{BrickCacheConfig, Stage};
use super::evict::EvictParams;
use super::evict::pass::{EvictPass, evict_layout_entries};
use super::feedback::ring::ReadbackRing;
use super::feedback::{Request, Victim, decode_requests, decode_victims};
use super::pool::gpu::GpuPools;
use super::pool::layout::{BufferLimits, PoolLayout};
use super::stream::batch::UploadBatch;
use super::stream::feed::{FeedFrame, VoxelWorldFeed};
use super::stream::frame::{FrameInput, FrameReport, run_frame};
use super::stream::key::Shape;
use super::stream::residency::Residency;
use super::stream::select::ViewPoint;
use super::stream::upload::{
    APPLY_ENTRY_POINTS, ApplyPipelines, UploadBuffer, apply_bind_group, apply_layout_entries,
    record_apply,
};
use crate::tier::RenderTier;

pub const APPLY_SHADER_PATH: &str = "shaders/brick_cache/apply.wesl";
pub const EVICT_SHADER_PATH: &str = "shaders/brick_cache/evict.wesl";

/// The feed taken from the main world and not processed yet.
#[derive(Resource, Default)]
pub struct PendingFeed(pub FeedFrame);

/// The config as the render world sees it this frame.
#[derive(Resource, Clone, Default)]
pub struct ExtractedCacheConfig(pub BrickCacheConfig);

/// Whether the config the render world holds is the one to size the pools
/// from: the file has been applied, or there is no file to wait for (no
/// config registry, as in tests), or [`CONFIG_GRACE_SECS`] have passed. The
/// pools never grow, so sizing them from the defaults before the file is in
/// would ignore the file's pool sizes for the whole run.
#[derive(Resource, Clone, Copy, Default)]
pub struct CacheConfigReady(pub bool);

/// How long to wait for `brick_cache.ron` before sizing the pools from the
/// defaults (a missing or broken file never blocks rendering).
pub const CONFIG_GRACE_SECS: f32 = 3.0;

/// The main camera's position, for stage 1 and edit priorities.
#[derive(Resource, Clone, Copy, Default)]
pub struct CacheViewPoint(pub Option<ViewPoint>);

/// Takes the feed (exactly once per change) and copies the config.
pub fn extract_feed(
    mut main_world: ResMut<MainWorld>,
    mut pending: ResMut<PendingFeed>,
    mut config: ResMut<ExtractedCacheConfig>,
    mut ready: ResMut<CacheConfigReady>,
) {
    if let Some(mut feed) = main_world.get_resource_mut::<VoxelWorldFeed>() {
        let taken = feed.take();
        pending.0.merge(taken);
    }
    if let Some(c) = main_world.get_resource::<Config<BrickCacheConfig>>() {
        config.0 = (**c).clone();
    }
    if !ready.0 {
        let applied = match main_world.get_resource::<ConfigRegistry>() {
            Some(registry) => registry
                .get(<BrickCacheConfig as ConfigAsset>::NAME)
                .is_some_and(|e| e.origin != ConfigOrigin::Defaults),
            None => true,
        };
        let waited = main_world
            .get_resource::<Time>()
            .is_some_and(|t| t.elapsed_secs() >= CONFIG_GRACE_SECS);
        ready.0 = applied || waited;
    }
}

/// Voxels per metre: Bevy's world unit is the metre.
pub const VOXELS_PER_METER: f32 = world_core::consts::VOXELS_PER_METER as f32;

/// The first perspective camera (the lowest order) as a stage-1 view point.
/// Orthographic views (UI, debug overlays) are skipped.
pub fn update_view_point(
    views: Query<(&ExtractedView, &ExtractedCamera)>,
    tier: Option<Res<RenderTier>>,
    mut point: ResMut<CacheViewPoint>,
) {
    let main = views
        .iter()
        .filter(|(v, _)| v.clip_from_view.w_axis.w == 0.0)
        .min_by_key(|(_, c)| c.order);
    point.0 = main.map(|(v, _)| {
        let eye = v.world_from_view.translation() * VOXELS_PER_METER;
        // Detail follows the traced height, not the window's.
        let viewport = bevy::math::UVec2::new(v.viewport.z, v.viewport.w);
        let internal = tier.as_ref().map(|t| t.budgets.internal_resolution);
        let height = crate::passes::primary::trace_size(viewport, internal).y as f32;
        // clip_from_view[1][1] is 1 / tan(fov / 2) for a perspective view.
        let f = v.clip_from_view.y_axis.y.abs().max(1e-6);
        ViewPoint {
            eye: eye.to_array(),
            pixel_angle: 2.0 / (f * height),
        }
    });
}

/// The pipelines the cache runs itself.
#[derive(Resource)]
pub struct CachePipelines {
    pub apply_layout: BindGroupLayoutDescriptor,
    pub evict_layout: BindGroupLayoutDescriptor,
    pub apply: [CachedComputePipelineId; 4],
    pub evict: CachedComputePipelineId,
}

pub fn init_cache_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
) {
    let apply_layout = BindGroupLayoutDescriptor::new("brick_cache.apply", &apply_layout_entries());
    let evict_layout = BindGroupLayoutDescriptor::new("brick_cache.evict", &evict_layout_entries());
    let apply_shader = assets.load(APPLY_SHADER_PATH);
    let evict_shader = assets.load(EVICT_SHADER_PATH);
    let compute = |label: &'static str,
                   layout: &BindGroupLayoutDescriptor,
                   shader: &Handle<Shader>,
                   entry: &'static str| {
        cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            immediate_size: 0,
            shader: shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some(entry.into()),
            zero_initialize_workgroup_memory: true,
            constants: Vec::new(),
        })
    };
    let apply = APPLY_ENTRY_POINTS.map(|e| compute(e, &apply_layout, &apply_shader, e));
    let evict = compute("brick_cache.evict", &evict_layout, &evict_shader, "evict");
    commands.insert_resource(CachePipelines {
        apply_layout,
        evict_layout,
        apply,
        evict,
    });
}

/// Everything the cache owns in the render world.
#[derive(Resource)]
pub struct BrickCacheState {
    pub pools: GpuPools,
    residency: Residency,
    belt: StagingBelt,
    upload: UploadBuffer,
    apply_bind_group: Option<(u64, wgpu::BindGroup)>,
    evict: EvictPass,
    ring: ReadbackRing,
    batch: UploadBatch,
    words: Vec<u32>,
    requests: Vec<Request>,
    victims: Vec<Victim>,
    snapshot: Option<Arc<TreeSnapshot>>,
    evicted_this_frame: bool,
    /// Frame number stamped on uploads and compared by rays (1 or more).
    pub frame: u32,
    pub last_report: FrameReport,
}

/// Creates the pools once the pipelines' layouts exist. Pool sizes come from
/// the config as it is at this moment (the shipped file's values are also
/// the defaults) and never change afterwards.
pub fn init_brick_cache(
    mut commands: Commands,
    state: Option<Res<BrickCacheState>>,
    pipelines: Option<Res<CachePipelines>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    config: Res<ExtractedCacheConfig>,
    ready: Res<CacheConfigReady>,
    tier: Option<Res<RenderTier>>,
) {
    if state.is_some() || !ready.0 {
        return;
    }
    let Some(pipelines) = pipelines else {
        return;
    };
    let wgpu_device = device.wgpu_device();
    let profile = tier.as_ref().map(|t| t.profile).unwrap_or_default();
    let mut sizes = config.0.sizes(profile);
    if let Some(t) = &tier {
        let cap_mib = (t.budgets.brick_pool_bytes / (1024 * 1024)).max(16) as u32;
        sizes.brick_pool_mib = sizes.brick_pool_mib.min(cap_mib);
    }
    let layout = PoolLayout::plan(sizes, &config.0, BufferLimits::from_wgpu(&device.limits()));
    info!(
        "brick cache: {} MiB nodes, {} MiB bricks in {} segment(s), {} brick ids, {} MiB upload budget, stage {:?}",
        layout.node_pool_bytes() >> 20,
        layout.brick_pool_bytes() >> 20,
        layout.segments.len(),
        layout.brick_ids,
        layout.upload_budget >> 20,
        config.0.stage,
    );
    let shape = Shape {
        levels: world_core::consts::TREE_DEPTH,
        origin: world_core::VoxelPos::new(
            world_core::consts::WORLD_MIN_VOXEL,
            world_core::consts::WORLD_MIN_VOXEL,
            world_core::consts::WORLD_MIN_VOXEL,
        ),
    };
    let Some(residency) = Residency::new(shape, &layout) else {
        error!("brick cache: the node allocator could not reserve the root record");
        return;
    };
    let pools = GpuPools::new(wgpu_device, layout.clone());
    let evict_layout = cache.get_bind_group_layout(&pipelines.evict_layout);
    let evict = EvictPass::new(wgpu_device, &evict_layout, &pools);
    let ring = ReadbackRing::new(
        wgpu_device,
        layout.request_block_words(),
        layout.victim_block_words(),
    );
    let chunk = layout.upload_budget.clamp(1 << 20, 64 << 20);
    commands.insert_resource(BrickCacheState {
        upload: UploadBuffer::new(wgpu_device, chunk),
        belt: StagingBelt::new(wgpu_device.clone(), chunk),
        pools,
        residency,
        apply_bind_group: None,
        evict,
        ring,
        batch: UploadBatch::default(),
        words: Vec::new(),
        requests: Vec::new(),
        victims: Vec::new(),
        snapshot: None,
        evicted_this_frame: false,
        frame: 0,
        last_report: FrameReport::default(),
    });
}

/// Starts a render frame: the number every pass of this frame uses.
pub fn advance_frame(state: Option<ResMut<BrickCacheState>>) {
    if let Some(mut state) = state {
        state.frame = state.frame.wrapping_add(1).max(1);
        state.evicted_this_frame = false;
    }
}

/// One frame of streaming, recorded ahead of every camera.
pub fn run_brick_cache(
    state: Option<ResMut<BrickCacheState>>,
    pipelines: Option<Res<CachePipelines>>,
    cache: Res<PipelineCache>,
    mut pending: ResMut<PendingFeed>,
    config: Res<ExtractedCacheConfig>,
    view: Res<CacheViewPoint>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) {
    let (Some(mut state), Some(pipelines)) = (state, pipelines) else {
        return;
    };
    let state = &mut *state;
    let wgpu_device = device.wgpu_device();
    let shape = state.residency.shape();
    let layout = state.pools.layout.clone();

    for readback in state.ring.collect(wgpu_device) {
        let (requests, _) = decode_requests(&readback.requests, layout.request_capacity, shape);
        state.requests = requests;
        if !readback.victims.is_empty() {
            let (victims, _) =
                decode_victims(&readback.victims, layout.victim_capacity, layout.brick_ids);
            state.victims.extend(victims);
        }
    }

    let apply: Option<Vec<&bevy::render::render_resource::ComputePipeline>> = pipelines
        .apply
        .iter()
        .map(|&id| cache.get_compute_pipeline(id))
        .collect();
    let (Some(apply), Some(evict_pipeline)) = (apply, cache.get_compute_pipeline(pipelines.evict))
    else {
        return; // Shaders still compiling: keep the feed pending.
    };

    let feed = std::mem::take(&mut pending.0);
    let mut refresh = feed.refresh;
    if feed.snapshot.is_some() {
        if state.snapshot.is_none() {
            refresh = true;
        }
        state.snapshot = feed.snapshot;
    } else if feed.refresh {
        state.snapshot = None;
    }
    let empty;
    let source: &TreeSnapshot = match &state.snapshot {
        Some(s) => s,
        None if refresh => {
            empty = VoxelTree::new().snapshot();
            &empty
        }
        None => return,
    };
    let requests = if config.0.stage == Stage::RayGuided {
        std::mem::take(&mut state.requests)
    } else {
        state.requests.clear();
        Vec::new()
    };
    let victims = std::mem::take(&mut state.victims);
    let input = FrameInput {
        source,
        frame: state.frame,
        refresh,
        dirty: &feed.dirty,
        view: view.0,
        requests: &requests,
        victims: &victims,
        config: &config.0,
        budget: layout.upload_budget,
        victim_capacity: layout.victim_capacity,
    };
    let report = run_frame(&mut state.residency, &input, &mut state.batch);
    state.last_report = report;

    let mut encoder = wgpu_device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("brick_cache.frame"),
    });
    if !state.batch.is_empty() {
        let counts = state.batch.serialize(&mut state.words);
        state
            .upload
            .reserve(wgpu_device, state.words.len() as u64 * 4);
        let generation = state.upload.generation;
        if state.apply_bind_group.as_ref().map(|(g, _)| *g) != Some(generation) {
            let layout = cache.get_bind_group_layout(&pipelines.apply_layout);
            let bg = apply_bind_group(wgpu_device, &layout, &state.pools, &state.upload);
            state.apply_bind_group = Some((generation, bg));
        }
        if let Some((_, bind_group)) = &state.apply_bind_group {
            let apply_pipelines = ApplyPipelines {
                unlink: apply[0],
                scatter: apply[1],
                table: apply[2],
                link: apply[3],
            };
            record_apply(
                &mut encoder,
                &mut state.belt,
                &state.upload,
                &state.words,
                counts,
                bind_group,
                &apply_pipelines,
            );
        }
    }
    if let Some(target) = report.evict_target
        && !state.ring.victims_in_flight()
    {
        let params = EvictParams {
            frame: state.frame,
            min_age: config.0.min_evict_age_frames,
            target,
            id_count: layout.brick_ids,
            capacity: layout.victim_capacity,
            _pad: [0; 3],
        };
        state
            .evict
            .record(&queue, &mut encoder, &state.pools, evict_pipeline, &params);
        state.evicted_this_frame = true;
    }
    state.belt.finish_and_recall_on_submit(&encoder);
    ctx.add_command_buffer(encoder.finish());
}

/// Copies this frame's requests (and victims) to a read-back slot and clears
/// the request counter. Runs after every camera traced.
pub fn copy_feedback(state: Option<ResMut<BrickCacheState>>, mut ctx: RenderContext) {
    let Some(mut state) = state else {
        return;
    };
    let state = &mut *state;
    let encoder = ctx.command_encoder();
    let victims = state.evicted_this_frame.then_some(&state.pools.victims);
    state
        .ring
        .record_copy(encoder, &state.pools.feedback, victims, state.frame);
    let header = u64::from(super::pool::layout::FEEDBACK_HEADER_WORDS) * 4;
    encoder.clear_buffer(&state.pools.feedback, 0, Some(header));
}

/// Maps the slots copied this frame, after submission.
pub fn map_feedback(state: Option<ResMut<BrickCacheState>>) {
    if let Some(mut state) = state {
        state.ring.after_submit();
    }
}

/// Logs cache figures every few seconds.
pub fn log_cache_stats(
    state: Option<Res<BrickCacheState>>,
    view: Option<Res<CacheViewPoint>>,
    mut last: Local<u32>,
) {
    let Some(state) = state else {
        return;
    };
    // Frames 1 and 100, then every 1000.
    let due = state.frame == 1 || state.frame == 100 || state.frame.is_multiple_of(1000);
    if !due || state.frame == *last {
        return;
    }
    *last = state.frame;
    let s = state.residency.stats();
    let r = state.last_report;
    info!(
        "brick cache frame {}: {} records, {} expanded, {} bricks ({} KiB, {:.1}% bricks, {:.1}% nodes); last frame: {} loaded, {} expanded, {} edits uploaded, {} unlinked, {} evicted, {} victims, {} B upload; read-back skips {}; detail x{:.2}",
        state.frame,
        s.entries,
        s.expanded,
        s.bricks,
        s.brick_bytes >> 10,
        s.brick_fill * 100.0,
        s.node_fill * 100.0,
        r.loaded,
        r.expanded,
        r.edited_uploaded,
        r.edited_unlinked,
        r.evicted,
        r.victims_applied,
        r.upload_bytes,
        state.ring.skipped,
        state.residency.pressure(),
    );
    if let Some(v) = view.as_ref().and_then(|v| v.0) {
        info!(
            "brick cache view: eye {:?} voxels, {:.2e} rad per pixel",
            v.eye, v.pixel_angle
        );
    }
}
