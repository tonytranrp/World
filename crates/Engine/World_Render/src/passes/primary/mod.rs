//! The primary-ray pass: one compute ray per pixel through the GPU pools,
//! written as integer hit records (`gbuffer`), after a beam prepass that
//! finds where each 8x8 tile's rays may start (decision 0023). `trace`
//! records both on bare wgpu; this module runs them for every 3D camera in
//! Bevy's render world.

pub mod trace;

use bevy::math::{Mat4, UVec2};
use bevy::prelude::*;
use bevy::render::render_resource::{
    BindGroupLayoutDescriptor, CachedComputePipelineId, ComputePipelineDescriptor, PipelineCache,
};
use bevy::render::renderer::{FlushCommands, RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::ExtractedView;

use crate::brick_cache::render::{BrickCacheState, ExtractedCacheConfig, VOXELS_PER_METER};
use crate::gbuffer::HIT_WORDS;
use crate::gpu_timing::{GpuPassTimer, SampledPassTimer, TimedPass};
use crate::tier::RenderTier;
use trace::{
    DEFAULT_MAX_ITERATIONS, MODE_CAMERA, TraceParams, beam_buffer, record_beam, record_trace,
    trace_bind_group, trace_layout_entries,
};

/// Asset path of the trace shader.
pub const TRACE_SHADER_PATH: &str = "shaders/passes/primary/trace.wesl";
/// Asset path of the beam prepass shader.
pub const BEAM_SHADER_PATH: &str = "shaders/passes/primary/beam.wesl";

#[derive(Resource)]
pub struct TracePipeline {
    pub layout: BindGroupLayoutDescriptor,
    pub id: CachedComputePipelineId,
    /// The beam prepass, on the same layout.
    pub beam: CachedComputePipelineId,
}

pub fn init_trace_pipeline(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new("primary.trace", &trace_layout_entries());
    let id = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("primary.trace".into()),
        layout: vec![layout.clone()],
        immediate_size: 0,
        shader: assets.load(TRACE_SHADER_PATH),
        shader_defs: Vec::new(),
        entry_point: Some("trace".into()),
        zero_initialize_workgroup_memory: trace::ZERO_INIT_WORKGROUP_MEMORY,
        constants: Vec::new(),
    });
    let beam = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("primary.beam".into()),
        layout: vec![layout.clone()],
        immediate_size: 0,
        shader: assets.load(BEAM_SHADER_PATH),
        shader_defs: Vec::new(),
        entry_point: Some("beam".into()),
        zero_initialize_workgroup_memory: false,
        constants: Vec::new(),
    });
    commands.insert_resource(TracePipeline { layout, id, beam });
}

/// The size a view is traced at: the viewport, scaled down (keeping its
/// aspect) so its height is at most the profile's internal height. The
/// composite stretches the result over the viewport.
pub fn trace_size(viewport: UVec2, internal: Option<UVec2>) -> UVec2 {
    let viewport = viewport.max(UVec2::ONE);
    let Some(internal) = internal else {
        return viewport;
    };
    let height = internal.y.max(1);
    if viewport.y <= height {
        return viewport;
    }
    let width = (u64::from(viewport.x) * u64::from(height) + u64::from(viewport.y) / 2)
        / u64::from(viewport.y);
    UVec2::new((width as u32).max(1), height)
}

/// One view's hit records, parameters and bind group.
#[derive(Component)]
pub struct VoxelViewTargets {
    /// The G-buffer: [`HIT_WORDS`] words per traced pixel.
    pub hits: wgpu::Buffer,
    pub params: wgpu::Buffer,
    /// The traced size (see [`trace_size`]).
    pub size: UVec2,
    /// The viewport the trace is stretched over.
    pub viewport: UVec2,
    pub bind_group: wgpu::BindGroup,
    /// Changes whenever `hits` is replaced, so dependents rebuild.
    pub generation: u64,
}

fn hits_buffer(device: &wgpu::Device, size: UVec2) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("primary.hits"),
        size: u64::from(size.x.max(1)) * u64::from(size.y.max(1)) * u64::from(HIT_WORDS) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// Creates or resizes every view's targets and writes its parameters.
pub fn prepare_view_targets(
    mut commands: Commands,
    mut views: Query<(Entity, &ExtractedView, Option<&mut VoxelViewTargets>)>,
    state: Option<Res<BrickCacheState>>,
    pipeline: Option<Res<TracePipeline>>,
    cache: Res<PipelineCache>,
    config: Res<ExtractedCacheConfig>,
    tier: Option<Res<RenderTier>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut generation: Local<u64>,
) {
    let (Some(state), Some(pipeline)) = (state, pipeline) else {
        return;
    };
    let wgpu_device = device.wgpu_device();
    for (entity, view, targets) in &mut views {
        let viewport = UVec2::new(view.viewport.z.max(1), view.viewport.w.max(1));
        let internal = tier.as_ref().map(|t| t.budgets.internal_resolution);
        let size = trace_size(viewport, internal);
        let clip_from_world = view
            .clip_from_world
            .unwrap_or_else(|| view.clip_from_view * view.world_from_view.to_matrix().inverse());
        let f = view.clip_from_view.y_axis.y.abs().max(1e-6);
        let eye = view.world_from_view.translation() * VOXELS_PER_METER;
        let o = world_core::consts::WORLD_MIN_VOXEL;
        let layout = &state.pools.layout;
        let params = TraceParams {
            clip_from_world: clip_from_world.to_cols_array_2d(),
            world_from_clip: Mat4::inverse(&clip_from_world).to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 0.0],
            origin: [o, o, o, world_core::consts::TREE_DEPTH as i32 - 1],
            size: [size.x, size.y, state.frame.max(1), MODE_CAMERA],
            lod: [
                2.0 / (f * size.y as f32),
                config.0.lod.footprint_bias,
                VOXELS_PER_METER,
                0.0,
            ],
            limits: [
                layout.request_capacity,
                layout.stamps_offset(),
                DEFAULT_MAX_ITERATIONS,
                config.0.lod.beam_iterations,
            ],
        };
        match targets {
            Some(mut t) if t.size == size => {
                queue.write_buffer(&t.params, 0, bytemuck::bytes_of(&params));
                if t.viewport != viewport {
                    t.viewport = viewport;
                }
            }
            _ => {
                let hits = hits_buffer(wgpu_device, size);
                // The prepass's start distances; the bind group keeps it.
                let beam = beam_buffer(wgpu_device, size.x, size.y);
                let params_buf = wgpu_device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("primary.params"),
                    size: size_of::<TraceParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params));
                let bind_layout = cache.get_bind_group_layout(&pipeline.layout);
                let bind_group = trace_bind_group(
                    wgpu_device,
                    &bind_layout,
                    &state.pools,
                    &hits,
                    &params_buf,
                    &beam,
                );
                *generation += 1;
                commands.entity(entity).insert(VoxelViewTargets {
                    hits,
                    params: params_buf,
                    size,
                    viewport,
                    bind_group,
                    generation: *generation,
                });
            }
        }
    }
}

/// Runs the beam prepass and traces the view (Core3d, after the opaque
/// pass). The first view of a frame is timed (see `gpu_timing`). The
/// prepass runs whatever `beam_iterations` says (with 0 each tile writes 0
/// at once), so a config change needs no other state.
pub fn voxel_trace(
    view: ViewQuery<&VoxelViewTargets>,
    pipeline: Option<Res<TracePipeline>>,
    cache: Res<PipelineCache>,
    timer: Option<ResMut<GpuPassTimer>>,
    sampled: Option<ResMut<SampledPassTimer>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut flush: FlushCommands,
    mut ctx: RenderContext,
) {
    let targets = view.into_inner();
    let Some(pipeline) = pipeline else {
        return;
    };
    let (Some(trace), Some(beam)) = (
        cache.get_compute_pipeline(pipeline.id),
        cache.get_compute_pipeline(pipeline.beam),
    ) else {
        return;
    };
    let (width, height) = (targets.size.x, targets.size.y);
    let bind_group = &targets.bind_group;
    match sampled {
        Some(mut sampled) if sampled.wants(TimedPass::Trace) => {
            let device = device.wgpu_device();
            let encoder_for = |label| {
                device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) })
            };
            let mut encoder = encoder_for("primary.beam");
            record_beam(&mut encoder, beam, bind_group, width, height, None);
            sampled.time_alone(TimedPass::Beam, device, &queue, || flush.flush(), encoder);
            let mut encoder = encoder_for("primary.trace");
            record_trace(&mut encoder, trace, bind_group, width, height, None);
            sampled.time_alone(TimedPass::Trace, device, &queue, || {}, encoder);
            sampled.read_rays(device, &queue, &targets.hits);
        }
        _ => {
            let mut timer = timer;
            let writes = timer
                .as_deref_mut()
                .and_then(|t| t.compute_writes(TimedPass::Beam));
            record_beam(
                ctx.command_encoder(),
                beam,
                bind_group,
                width,
                height,
                writes,
            );
            let writes = timer
                .as_deref_mut()
                .and_then(|t| t.compute_writes(TimedPass::Trace));
            record_trace(
                ctx.command_encoder(),
                trace,
                bind_group,
                width,
                height,
                writes,
            );
        }
    }
}

#[cfg(test)]
mod trace_size_tests {
    use super::*;

    #[test]
    fn views_taller_than_the_internal_height_are_traced_smaller_at_their_aspect() {
        let floor = Some(UVec2::new(1280, 720));
        assert_eq!(
            trace_size(UVec2::new(2560, 1440), floor),
            UVec2::new(1280, 720)
        );
        assert_eq!(
            trace_size(UVec2::new(3024, 1964), floor),
            UVec2::new(1109, 720)
        );
        // At or below the internal height, and without a tier, the viewport.
        assert_eq!(
            trace_size(UVec2::new(1280, 720), floor),
            UVec2::new(1280, 720)
        );
        assert_eq!(
            trace_size(UVec2::new(640, 360), floor),
            UVec2::new(640, 360)
        );
        assert_eq!(
            trace_size(UVec2::new(2560, 1440), None),
            UVec2::new(2560, 1440)
        );
        assert_eq!(trace_size(UVec2::ZERO, floor), UVec2::ONE);
    }
}
