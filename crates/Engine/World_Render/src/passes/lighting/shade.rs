//! Lighting v0: hit records to the lit target, one compute thread per
//! traced pixel.
//!
//! Each hit gets its material's colour times a fixed light per face. The lit
//! target is `Rgba32Float` at the trace's size: the linear colour in rgb and
//! the hit's depth (reverse Z) in alpha, or alpha -1 for a miss. Full `f32`
//! keeps the composite's output bit-identical to shading per screen pixel,
//! and puts colour and depth in one load for the composite.

use bevy::math::UVec2;
use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::render_resource::{
    BindGroupLayoutDescriptor, CachedComputePipelineId, ComputePipelineDescriptor, PipelineCache,
};
use bevy::render::renderer::{FlushCommands, RenderContext, RenderDevice, RenderQueue, ViewQuery};
use wgpu::{
    BindGroupLayoutEntry, BindingType, BufferBindingType, ShaderStages, StorageTextureAccess,
    TextureFormat, TextureViewDimension,
};
use world_core::MaterialTable;
use world_reload::Config;

use crate::brick_cache::stream::upload::storage_entry;
use crate::gpu_timing::{GpuPassTimer, SampledPassTimer, TimedPass};
use crate::passes::primary::VoxelViewTargets;

/// Asset path of the shading shader.
pub const SHADE_SHADER_PATH: &str = "shaders/passes/lighting/shade.wesl";
/// The shader's workgroup edge: 8x8 pixels.
pub const SHADE_WORKGROUP: u32 = 8;
/// Format of the lit target: colour in rgb, depth in alpha (see the module).
pub const LIT_FORMAT: TextureFormat = TextureFormat::Rgba32Float;

#[derive(Resource)]
pub struct ShadePipeline {
    pub layout: BindGroupLayoutDescriptor,
    pub id: CachedComputePipelineId,
}

fn layout_entries() -> Vec<BindGroupLayoutEntry> {
    vec![
        storage_entry(0, true, ShaderStages::COMPUTE),
        storage_entry(1, true, ShaderStages::COMPUTE),
        BindGroupLayoutEntry {
            binding: 2,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 3,
            visibility: ShaderStages::COMPUTE,
            ty: BindingType::StorageTexture {
                access: StorageTextureAccess::WriteOnly,
                format: LIT_FORMAT,
                view_dimension: TextureViewDimension::D2,
            },
            count: None,
        },
    ]
}

pub fn init_shade_pipeline(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    assets: Res<AssetServer>,
) {
    let layout = BindGroupLayoutDescriptor::new("lighting.shade", &layout_entries());
    let id = cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("lighting.shade".into()),
        layout: vec![layout.clone()],
        immediate_size: 0,
        shader: assets.load(SHADE_SHADER_PATH),
        shader_defs: Vec::new(),
        entry_point: Some("shade".into()),
        zero_initialize_workgroup_memory: false,
        constants: Vec::new(),
    });
    commands.insert_resource(ShadePipeline { layout, id });
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
                label: Some("lighting.material_colors"),
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

/// One view's lit target and the shading pass's bind group, rebuilt only
/// when what they refer to changes.
#[derive(Component)]
pub struct VoxelLitTarget {
    /// The lit target, [`LIT_FORMAT`] at the trace's size.
    pub view: wgpu::TextureView,
    /// The traced size, which the lit target has.
    pub size: UVec2,
    /// Changes whenever the lit target is replaced, so dependents rebind.
    pub generation: u64,
    bind_group: wgpu::BindGroup,
    key: (u64, u64, u32),
    _texture: wgpu::Texture,
    _params: wgpu::Buffer,
}

pub fn prepare_shade(
    mut commands: Commands,
    views: Query<(Entity, &VoxelViewTargets, Option<&VoxelLitTarget>)>,
    pipeline: Option<Res<ShadePipeline>>,
    colors: Option<Res<MaterialColorBuffer>>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut generation: Local<u64>,
) {
    let (Some(pipeline), Some(colors)) = (pipeline, colors) else {
        return;
    };
    let wgpu_device = device.wgpu_device();
    for (entity, targets, existing) in &views {
        let key = (targets.generation, colors.generation, colors.count);
        if existing.is_some_and(|e| e.key == key) {
            continue;
        }
        let size = targets.size;
        let texture = wgpu_device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lighting.lit"),
            size: wgpu::Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LIT_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let params = wgpu_device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting.shade_params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &params,
            0,
            bytemuck::cast_slice(&[size.x, size.y, colors.count, 0u32]),
        );
        let layout = cache.get_bind_group_layout(&pipeline.layout);
        let bind_group = wgpu_device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lighting.shade"),
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
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        *generation += 1;
        commands.entity(entity).insert(VoxelLitTarget {
            view,
            size,
            generation: *generation,
            bind_group,
            key,
            _texture: texture,
            _params: params,
        });
    }
}

/// Lights the view's hits into its lit target (Core3d, after the trace).
/// The first view of a frame is timed (see `gpu_timing`).
pub fn voxel_shade(
    view: ViewQuery<&VoxelLitTarget>,
    pipeline: Option<Res<ShadePipeline>>,
    cache: Res<PipelineCache>,
    timer: Option<ResMut<GpuPassTimer>>,
    sampled: Option<ResMut<SampledPassTimer>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut flush: FlushCommands,
    mut ctx: RenderContext,
) {
    let lit = view.into_inner();
    let Some(pipeline) = pipeline.and_then(|p| cache.get_compute_pipeline(p.id)) else {
        return;
    };
    let record = |encoder: &mut wgpu::CommandEncoder,
                  timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>| {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("lighting.shade"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &lit.bind_group, &[]);
        pass.dispatch_workgroups(
            lit.size.x.div_ceil(SHADE_WORKGROUP),
            lit.size.y.div_ceil(SHADE_WORKGROUP),
            1,
        );
    };
    match sampled {
        Some(mut sampled) if sampled.wants(TimedPass::Shade) => {
            let device = device.wgpu_device();
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lighting.shade"),
            });
            record(&mut encoder, None);
            sampled.time_alone(TimedPass::Shade, device, &queue, || flush.flush(), encoder);
        }
        _ => {
            let mut timer = timer;
            let writes = timer
                .as_deref_mut()
                .and_then(|t| t.compute_writes(TimedPass::Shade));
            record(ctx.command_encoder(), writes);
        }
    }
}
