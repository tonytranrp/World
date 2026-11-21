//! Composite, version 2: the lit voxel image to colour and depth.
//!
//! Lighting (`passes::lighting`) writes the lit target at the trace's
//! (internal) resolution. A fullscreen pass right after it stretches that
//! target over the viewport, each screen pixel loading the traced pixel
//! under it (nearest), and writes colour and depth with a `GreaterEqual`
//! depth test (reverse Z), so voxels and rasterized geometry occlude each
//! other both ways. Covered pixels get alpha 0 so the sky composite, which
//! runs later, leaves them alone; misses are discarded and keep the cleared
//! background. The pass does no shading: its cost per screen pixel is one
//! texture load and the colour and depth writes (decision 0017).

use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    BindGroupLayoutDescriptor, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    CompareFunction, DepthStencilState, FragmentState, MultisampleState, PipelineCache,
    RenderPipelineDescriptor, TextureFormat, VertexState,
};
use bevy::render::renderer::{FlushCommands, RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget};
use wgpu::{
    BindGroupLayoutEntry, BindingType, BufferBindingType, ShaderStages, StoreOp, TextureSampleType,
    TextureViewDimension,
};

use crate::gpu_timing::{GpuPassTimer, SampledPassTimer, TimedPass};
use crate::passes::lighting::shade::VoxelLitTarget;
use crate::passes::primary::VoxelViewTargets;

/// Asset path of the voxel composite shader.
pub const VOXEL_COMPOSITE_SHADER_PATH: &str = "shaders/passes/composite/voxel.wesl";

#[derive(Resource)]
pub struct VoxelCompositePipeline {
    pub layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
    vertex: VertexState,
    variants: HashMap<(TextureFormat, u32), CachedRenderPipelineId>,
}

fn layout_entries() -> Vec<BindGroupLayoutEntry> {
    vec![
        BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 1,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
    ]
}

pub fn init_composite_pipeline(
    mut commands: Commands,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
) {
    commands.insert_resource(VoxelCompositePipeline {
        layout: BindGroupLayoutDescriptor::new("composite.voxel", &layout_entries()),
        shader: assets.load(VOXEL_COMPOSITE_SHADER_PATH),
        vertex: fullscreen.to_vertex_state(),
        variants: HashMap::default(),
    });
}

impl VoxelCompositePipeline {
    fn variant(
        &mut self,
        cache: &PipelineCache,
        format: TextureFormat,
        samples: u32,
    ) -> CachedRenderPipelineId {
        *self.variants.entry((format, samples)).or_insert_with(|| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("composite.voxel".into()),
                layout: vec![self.layout.clone()],
                vertex: self.vertex.clone(),
                fragment: Some(FragmentState {
                    shader: self.shader.clone(),
                    entry_point: Some("fragment".into()),
                    targets: vec![Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                depth_stencil: Some(DepthStencilState {
                    format: CORE_3D_DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(CompareFunction::GreaterEqual),
                    stencil: default(),
                    bias: default(),
                }),
                multisample: MultisampleState {
                    count: samples,
                    ..default()
                },
                ..default()
            })
        })
    }
}

/// One view's composite pipeline and bind group, rebuilt only when what
/// they refer to changes.
#[derive(Component)]
pub struct VoxelCompositeView {
    pipeline: CachedRenderPipelineId,
    bind_group: wgpu::BindGroup,
    key: (u64, UVec2),
    _params: wgpu::Buffer,
}

pub fn prepare_composite(
    mut commands: Commands,
    views: Query<(
        Entity,
        &ExtractedView,
        &VoxelViewTargets,
        &VoxelLitTarget,
        Option<&Msaa>,
        Option<&VoxelCompositeView>,
    )>,
    pipeline: Option<ResMut<VoxelCompositePipeline>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(mut pipeline) = pipeline else {
        return;
    };
    let wgpu_device = device.wgpu_device();
    for (entity, view, targets, lit, msaa, existing) in &views {
        let samples = msaa.map_or(1, Msaa::samples);
        let id = pipeline.variant(&cache, view.target_format, samples);
        let key = (lit.generation, targets.viewport);
        if let Some(e) = existing
            && e.key == key
            && e.pipeline == id
        {
            continue;
        }
        let params = wgpu_device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite.voxel_params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let p = [
            lit.size.x,
            lit.size.y,
            0u32,
            0,
            targets.viewport.x,
            targets.viewport.y,
            0,
            0,
        ];
        queue.write_buffer(&params, 0, bytemuck::cast_slice(&p));
        let layout = cache.get_bind_group_layout(&pipeline.layout);
        let bind_group = wgpu_device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite.voxel"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&lit.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        commands.entity(entity).insert(VoxelCompositeView {
            pipeline: id,
            bind_group,
            key,
            _params: params,
        });
    }
}

/// Draws the voxels into the view's colour and depth (Core3d, after
/// lighting). The first view of a frame is timed (see `gpu_timing`).
pub fn voxel_composite(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &VoxelCompositeView)>,
    cache: Res<PipelineCache>,
    timer: Option<ResMut<GpuPassTimer>>,
    sampled: Option<ResMut<SampledPassTimer>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut flush: FlushCommands,
    mut ctx: RenderContext,
) {
    let (target, depth, composite) = view.into_inner();
    let Some(pipeline) = cache.get_render_pipeline(composite.pipeline) else {
        return;
    };
    let record = |encoder: &mut wgpu::CommandEncoder,
                  timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>| {
        let color = target.get_color_attachment();
        let depth = depth.get_attachment(StoreOp::Store);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("composite.voxel"),
            color_attachments: &[Some(color)],
            depth_stencil_attachment: Some(depth),
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &composite.bind_group, &[]);
        pass.draw(0..3, 0..1);
    };
    match sampled {
        Some(mut sampled) if sampled.wants(TimedPass::Composite) => {
            let device = device.wgpu_device();
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("composite.voxel"),
            });
            record(&mut encoder, None);
            sampled.time_alone(
                TimedPass::Composite,
                device,
                &queue,
                || flush.flush(),
                encoder,
            );
        }
        _ => {
            let mut timer = timer;
            let writes = timer
                .as_deref_mut()
                .and_then(|t| t.render_writes(TimedPass::Composite));
            record(ctx.command_encoder(), writes);
        }
    }
}
