//! Composite, version 1: voxel hits to colour and depth.
//!
//! A fullscreen pass right after Bevy's opaque pass reads each pixel's hit
//! record, shades it with the material colour and a fixed light per face,
//! and writes colour and depth with a `GreaterEqual` depth test (reverse Z),
//! so voxels and rasterized geometry occlude each other both ways. Covered
//! pixels get alpha 0 so the sky composite, which runs later, leaves them
//! alone; misses are discarded and keep the cleared background.

use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::render_resource::{
    BindGroupLayoutDescriptor, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    CompareFunction, DepthStencilState, FragmentState, MultisampleState, PipelineCache,
    RenderPipelineDescriptor, TextureFormat, VertexState,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::view::{ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget};
use wgpu::{BindGroupLayoutEntry, BindingType, BufferBindingType, ShaderStages, StoreOp};
use world_core::MaterialTable;
use world_reload::Config;

use crate::brick_cache::stream::upload::storage_entry;
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
        storage_entry(0, true, ShaderStages::FRAGMENT),
        storage_entry(1, true, ShaderStages::FRAGMENT),
        BindGroupLayoutEntry {
            binding: 2,
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

/// Linear material colours by id (index 0 is air), from the material table.
#[derive(Resource, Default, Clone, PartialEq)]
pub struct MaterialColors(pub Vec<[f32; 4]>);

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Copies the material table's colours when they change.
pub fn extract_material_colors(main_world: Res<MainWorld>, mut colors: ResMut<MaterialColors>) {
    let Some(table) = main_world.get_resource::<Config<MaterialTable>>() else {
        return;
    };
    let max = table.iter().map(|m| m.id.0).max().unwrap_or(0) as usize;
    let mut next = vec![[1.0, 0.0, 1.0, 1.0]; max + 1];
    next[0] = [0.0, 0.0, 0.0, 0.0];
    for m in table.iter() {
        let c = m.color;
        next[m.id.0 as usize] = [
            srgb_to_linear(c.0),
            srgb_to_linear(c.1),
            srgb_to_linear(c.2),
            1.0,
        ];
    }
    if colors.0 != next {
        colors.0 = next;
    }
}

/// The colour buffer on the GPU.
#[derive(Resource)]
pub struct MaterialColorBuffer {
    pub buffer: wgpu::Buffer,
    pub count: u32,
    /// Changes whenever `buffer` is replaced.
    pub generation: u64,
    uploaded: Vec<[f32; 4]>,
}

pub fn prepare_material_colors(
    mut commands: Commands,
    colors: Res<MaterialColors>,
    existing: Option<ResMut<MaterialColorBuffer>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut generation: Local<u64>,
) {
    let mut data = colors.0.clone();
    if data.is_empty() {
        data.push([1.0, 0.0, 1.0, 1.0]);
    }
    match existing {
        Some(mut b) if b.count as usize == data.len() => {
            if b.uploaded != data {
                queue.write_buffer(&b.buffer, 0, bytemuck::cast_slice(&data));
                b.uploaded = data;
            }
        }
        _ => {
            let buffer = device.wgpu_device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("composite.material_colors"),
                size: (data.len() * 16) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&data));
            *generation += 1;
            commands.insert_resource(MaterialColorBuffer {
                buffer,
                count: data.len() as u32,
                generation: *generation,
                uploaded: data,
            });
        }
    }
}

/// One view's composite pipeline and bind group, rebuilt only when what
/// they refer to changes.
#[derive(Component)]
pub struct VoxelCompositeView {
    pipeline: CachedRenderPipelineId,
    bind_group: wgpu::BindGroup,
    key: (u64, u64, u32),
    _params: wgpu::Buffer,
}

pub fn prepare_composite(
    mut commands: Commands,
    views: Query<(
        Entity,
        &ExtractedView,
        &VoxelViewTargets,
        Option<&Msaa>,
        Option<&VoxelCompositeView>,
    )>,
    pipeline: Option<ResMut<VoxelCompositePipeline>>,
    colors: Option<Res<MaterialColorBuffer>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let (Some(mut pipeline), Some(colors)) = (pipeline, colors) else {
        return;
    };
    let wgpu_device = device.wgpu_device();
    for (entity, view, targets, msaa, existing) in &views {
        let samples = msaa.map_or(1, Msaa::samples);
        let id = pipeline.variant(&cache, view.target_format, samples);
        let key = (targets.generation, colors.generation, colors.count);
        if let Some(e) = existing
            && e.key == key
            && e.pipeline == id
        {
            continue;
        }
        let params = wgpu_device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite.voxel_params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let p = [targets.size.x, targets.size.y, colors.count, 0u32];
        queue.write_buffer(&params, 0, bytemuck::cast_slice(&p));
        let layout = cache.get_bind_group_layout(&pipeline.layout);
        let bind_group = wgpu_device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite.voxel"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: targets.hits.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: colors.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
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

/// Draws the voxels into the view's colour and depth (Core3d, after the trace).
pub fn voxel_composite(
    view: ViewQuery<(&ViewTarget, &ViewDepthStencilTexture, &VoxelCompositeView)>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, depth, composite) = view.into_inner();
    let Some(pipeline) = cache.get_render_pipeline(composite.pipeline) else {
        return;
    };
    let color = target.get_color_attachment();
    let depth = depth.get_attachment(StoreOp::Store);
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("composite.voxel"),
            color_attachments: &[Some(color)],
            depth_stencil_attachment: Some(depth),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, &composite.bind_group, &[]);
    pass.draw(0..3, 0..1);
}
