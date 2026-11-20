//! Recording the eviction pass.

use wgpu::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingType, Buffer,
    BufferBindingType, BufferDescriptor, BufferUsages, CommandEncoder, ComputePassDescriptor,
    ComputePipeline, Device, Queue, ShaderStages,
};

use super::EvictParams;
use crate::brick_cache::pool::gpu::GpuPools;
use crate::brick_cache::pool::layout::FEEDBACK_HEADER_WORDS;
use crate::brick_cache::stream::upload::storage_entry;

/// Bindings: brick table (read), victim block (read-write), parameters.
pub fn evict_layout_entries() -> Vec<BindGroupLayoutEntry> {
    vec![
        storage_entry(0, true, ShaderStages::COMPUTE),
        storage_entry(1, false, ShaderStages::COMPUTE),
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
    ]
}

/// The pass's parameter buffer and bind group.
pub struct EvictPass {
    pub params: Buffer,
    pub bind_group: BindGroup,
}

impl EvictPass {
    pub fn new(device: &Device, layout: &BindGroupLayout, pools: &GpuPools) -> Self {
        let params = device.create_buffer(&BufferDescriptor {
            label: Some("brick_cache.evict_params"),
            size: size_of::<EvictParams>() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("brick_cache.evict"),
            layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: pools.table.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: pools.victims.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        Self { params, bind_group }
    }

    /// Clears the victim block and records the pass for `params`.
    pub fn record(
        &self,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        pools: &GpuPools,
        pipeline: &ComputePipeline,
        params: &EvictParams,
    ) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(params));
        encoder.clear_buffer(
            &pools.victims,
            0,
            Some(u64::from(FEEDBACK_HEADER_WORDS) * 4),
        );
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("brick_cache.evict"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}
