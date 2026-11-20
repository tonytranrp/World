//! The primary trace pass on bare wgpu: parameters, bindings and dispatch.
//! The Bevy node and the GPU tests both record through here.

use bytemuck::{Pod, Zeroable};
use wgpu::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingType, Buffer,
    BufferBindingType, CommandEncoder, ComputePassDescriptor, ComputePipeline, Device,
    ShaderStages,
};

use crate::brick_cache::pool::gpu::GpuPools;
use crate::brick_cache::stream::upload::storage_entry;

/// The shader's workgroup edge: 8x8 pixels.
pub const TRACE_WORKGROUP: u32 = 8;
/// Traversal iterations before a ray gives up (flagged exhausted).
pub const DEFAULT_MAX_ITERATIONS: u32 = 4096;
/// `mode`: rays from the camera matrices.
pub const MODE_CAMERA: u32 = 0;
/// `mode`: rays read from the hit buffer (tests).
pub const MODE_RAYS: u32 = 1;

/// The trace pass's uniform block. Every field is 16-byte aligned as WGSL
/// uniform layout requires.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct TraceParams {
    /// Clip from world (metres), for depth.
    pub clip_from_world: [[f32; 4]; 4],
    /// World (metres) from clip, for camera rays.
    pub world_from_clip: [[f32; 4]; 4],
    /// Camera position in voxels (world voxel frame); w unused.
    pub eye: [f32; 4],
    /// Tree origin in voxels; w is the root level.
    pub origin: [i32; 4],
    /// Width, height, frame (1 or more), mode.
    pub size: [u32; 4],
    /// Pixel angle (radians), footprint bias (0 is full resolution),
    /// voxels per metre, unused.
    pub lod: [f32; 4],
    /// Request capacity, word offset of the request stamps, iteration cap, unused.
    pub limits: [u32; 4],
}

/// Bindings: nodes, segments 0 to 3 (read), brick table and feedback
/// (read-write), hit records (read-write), parameters (uniform).
pub fn trace_layout_entries() -> Vec<BindGroupLayoutEntry> {
    let c = ShaderStages::COMPUTE;
    let mut v: Vec<_> = (0..5).map(|b| storage_entry(b, true, c)).collect();
    v.push(storage_entry(5, false, c));
    v.push(storage_entry(6, false, c));
    v.push(storage_entry(7, false, c));
    v.push(BindGroupLayoutEntry {
        binding: 8,
        visibility: c,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    });
    v
}

/// The bind group of one view: pools, its hit buffer and its parameters.
pub fn trace_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    pools: &GpuPools,
    hits: &Buffer,
    params: &Buffer,
) -> BindGroup {
    let mut buffers: Vec<&Buffer> = vec![&pools.nodes];
    buffers.extend(pools.segments.iter());
    buffers.extend([&pools.table, &pools.feedback, hits, params]);
    let entries: Vec<BindGroupEntry> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| BindGroupEntry {
            binding: i as u32,
            resource: b.as_entire_binding(),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("primary.trace"),
        layout,
        entries: &entries,
    })
}

/// Records the trace over `width x height` pixels.
pub fn record_trace(
    encoder: &mut CommandEncoder,
    pipeline: &ComputePipeline,
    bind_group: &BindGroup,
    width: u32,
    height: u32,
    timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
) {
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("primary.trace"),
        timestamp_writes: timestamps,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.dispatch_workgroups(
        width.div_ceil(TRACE_WORKGROUP),
        height.div_ceil(TRACE_WORKGROUP),
        1,
    );
}
