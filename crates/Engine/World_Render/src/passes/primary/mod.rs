//! The primary-ray pass: one compute ray per pixel through the GPU pools,
//! written as integer hit records (`gbuffer`). `trace` records it on bare
//! wgpu; this module runs it for every 3D camera in Bevy's render world.

pub mod trace;

use bevy::math::{Mat4, UVec2};
use bevy::prelude::*;
use bevy::render::render_resource::{
    BindGroupLayoutDescriptor, CachedComputePipelineId, ComputePipelineDescriptor, PipelineCache,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::ExtractedView;

use crate::brick_cache::render::{BrickCacheState, ExtractedCacheConfig, VOXELS_PER_METER};
use crate::gbuffer::HIT_WORDS;
use trace::{
    DEFAULT_MAX_ITERATIONS, MODE_CAMERA, TraceParams, record_trace, trace_bind_group,
    trace_layout_entries,
};

/// Asset path of the trace shader.
pub const TRACE_SHADER_PATH: &str = "shaders/passes/primary/trace.wesl";

#[derive(Resource)]
pub struct TracePipeline {
    pub layout: BindGroupLayoutDescriptor,
    pub id: CachedComputePipelineId,
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
        zero_initialize_workgroup_memory: true,
        constants: Vec::new(),
    });
    commands.insert_resource(TracePipeline { layout, id });
}

/// One view's hit records, parameters and bind group.
#[derive(Component)]
pub struct VoxelViewTargets {
    /// The G-buffer: [`HIT_WORDS`] words per pixel.
    pub hits: wgpu::Buffer,
    pub params: wgpu::Buffer,
    pub size: UVec2,
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
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut generation: Local<u64>,
) {
    let (Some(state), Some(pipeline)) = (state, pipeline) else {
        return;
    };
    let wgpu_device = device.wgpu_device();
    for (entity, view, targets) in &mut views {
        let size = UVec2::new(view.viewport.z.max(1), view.viewport.w.max(1));
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
                0,
            ],
        };
        match targets {
            Some(t) if t.size == size => {
                queue.write_buffer(&t.params, 0, bytemuck::bytes_of(&params));
            }
            _ => {
                let hits = hits_buffer(wgpu_device, size);
                let params_buf = wgpu_device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("primary.params"),
                    size: size_of::<TraceParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params));
                let bind_layout = cache.get_bind_group_layout(&pipeline.layout);
                let bind_group =
                    trace_bind_group(wgpu_device, &bind_layout, &state.pools, &hits, &params_buf);
                *generation += 1;
                commands.entity(entity).insert(VoxelViewTargets {
                    hits,
                    params: params_buf,
                    size,
                    bind_group,
                    generation: *generation,
                });
            }
        }
    }
}

/// Traces the view (Core3d, after the opaque pass).
pub fn voxel_trace(
    view: ViewQuery<&VoxelViewTargets>,
    pipeline: Option<Res<TracePipeline>>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let targets = view.into_inner();
    let Some(pipeline) = pipeline.and_then(|p| cache.get_compute_pipeline(p.id)) else {
        return;
    };
    record_trace(
        ctx.command_encoder(),
        pipeline,
        &targets.bind_group,
        targets.size.x,
        targets.size.y,
        None,
    );
}
