//! Render-graph wiring: where each voxel system runs. No pass logic here.
//!
//! Main world: the feed and the brick cache config, and the debug scene.
//!
//! Render world, per frame:
//!
//! | schedule / set | systems |
//! |----------------|---------|
//! | `ExtractSchedule` | take the feed, copy config and material colours |
//! | `Render`, after `ExtractCommands`, in no set | create the pools once, advance the frame number, the camera's view point, then one frame of streaming on the CPU (overlapping `prepare_windows`) |
//! | `Render` / `PrepareResources` | per-view hit buffers and trace parameters, material colours |
//! | `Render` / `PrepareBindGroups` | lit targets and shading bind groups, then composite pipelines and bind groups |
//! | `RenderGraph` / `Begin` | the streamed frame's upload, apply and eviction passes |
//! | `Core3d` / `MainPass`, after the opaque pass | trace, shade (internal resolution), then composite (colour and depth) |
//! | `RenderGraph` / `Render`, after `camera_driver` | copy requests and victims to a read-back slot, resolve the frame's GPU timestamps |
//! | `RenderGraph` / `Finish` | map the read-back slots and the timing slot |
//!
//! The voxel passes finish and write depth before Bevy's transparent pass,
//! gizmos and UI.

use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
use bevy::core_pipeline::schedule::camera_driver;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::renderer::{RenderGraph, RenderGraphSystems};
use bevy::render::{ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use world_reload::AppConfigExt;

use crate::brick_cache::config::BrickCacheConfig;
use crate::brick_cache::render::{
    CacheConfigReady, CacheViewPoint, ExtractedCacheConfig, PendingFeed, advance_frame,
    copy_feedback, extract_feed, init_brick_cache, init_cache_pipelines, log_cache_stats,
    map_feedback, run_brick_cache, stream_brick_cache, update_view_point,
};
use crate::brick_cache::stream::feed::VoxelWorldFeed;
use crate::debug::{start_debug_scene, update_debug_scene};
use crate::gpu_timing::{
    begin_gpu_timing, extract_gpu_timing, init_gpu_timer, map_gpu_timing, resolve_gpu_timing,
};
use crate::passes::composite::voxel::{
    init_composite_pipeline, prepare_composite, voxel_composite,
};
use crate::passes::lighting::shade::{
    MaterialColors, extract_material_colors, init_shade_pipeline, prepare_material_colors,
    prepare_shade, voxel_shade,
};
use crate::passes::primary::{init_trace_pipeline, prepare_view_targets, voxel_trace};
use crate::tier::RenderTier;

pub(crate) fn build(app: &mut App, tier: &RenderTier) {
    app.init_resource::<VoxelWorldFeed>()
        .add_config::<BrickCacheConfig>();

    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        // No renderer (headless tests): the feed still exists, nothing draws.
        return;
    };
    render_app
        .insert_resource(tier.clone())
        .init_resource::<PendingFeed>()
        .init_resource::<ExtractedCacheConfig>()
        .init_resource::<CacheConfigReady>()
        .init_resource::<CacheViewPoint>()
        .init_resource::<MaterialColors>()
        .add_systems(
            ExtractSchedule,
            (extract_feed, extract_material_colors, extract_gpu_timing),
        )
        .add_systems(
            RenderStartup,
            (
                init_cache_pipelines,
                init_trace_pipeline,
                init_shade_pipeline,
                init_composite_pipeline,
                init_gpu_timer,
            ),
        )
        .add_systems(
            Render,
            (
                // In no set, so that streaming overlaps `prepare_windows`
                // (its wait for a swapchain texture) instead of following it.
                (
                    init_brick_cache,
                    advance_frame,
                    update_view_point,
                    stream_brick_cache,
                )
                    .chain()
                    .after(RenderSystems::ExtractCommands)
                    .before(RenderSystems::Render),
                begin_gpu_timing.in_set(RenderSystems::Prepare),
                // The trace parameters carry this frame's number.
                prepare_view_targets
                    .after(advance_frame)
                    .in_set(RenderSystems::PrepareResources),
                prepare_material_colors.in_set(RenderSystems::PrepareResources),
                (prepare_shade, prepare_composite)
                    .chain()
                    .in_set(RenderSystems::PrepareBindGroups),
                log_cache_stats.in_set(RenderSystems::Cleanup),
            ),
        )
        .add_systems(
            RenderGraph,
            (
                run_brick_cache.in_set(RenderGraphSystems::Begin),
                copy_feedback
                    .in_set(RenderGraphSystems::Render)
                    .after(camera_driver),
                resolve_gpu_timing
                    .in_set(RenderGraphSystems::Render)
                    .after(camera_driver),
                (map_feedback, map_gpu_timing).in_set(RenderGraphSystems::Finish),
            ),
        )
        .add_systems(
            Core3d,
            (voxel_trace, voxel_shade, voxel_composite)
                .chain()
                .in_set(Core3dSystems::MainPass)
                .after(main_opaque_pass_3d)
                .before(main_transparent_pass_3d),
        );

    // The debug scene only matters when something renders it.
    app.add_systems(Startup, start_debug_scene)
        .add_systems(Update, update_debug_scene);
}
