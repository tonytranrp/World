//! The primary trace pass on bare wgpu: parameters, bindings and dispatch,
//! and its beam prepass (decision 0023). The Bevy node and the GPU tests
//! both record through here.

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
/// The beam prepass's tile edge in pixels: one beam per 8x8 camera rays.
pub const BEAM_TILE: u32 = 8;
/// The beam shader's workgroup edge, in tiles.
pub const BEAM_WORKGROUP: u32 = 8;
/// `limits.w` that turns the beam prepass off.
pub const BEAM_OFF: u32 = 0;

/// The constants the primary pass's shaders share with this module
/// (decision 0013).
pub const SHADER_CONSTANTS: &[(&str, u32)] = &[("BEAM_TILE", BEAM_TILE)];
/// Traversal iterations before a ray gives up (flagged exhausted).
pub const DEFAULT_MAX_ITERATIONS: u32 = 4096;
/// `mode`: rays from the camera matrices, written as the G-buffer's two
/// planes (`gbuffer`).
pub const MODE_CAMERA: u32 = 0;
/// `mode`: rays read from the hit buffer, written as full records (tests).
pub const MODE_RAYS: u32 = 1;
/// `mode`: rays from the camera matrices, written as full records (tests:
/// the same trace as [`MODE_CAMERA`], with the hit voxel kept).
pub const MODE_CAMERA_FULL: u32 = 2;
/// Whether the trace pipeline asks wgpu to zero its workgroup memory before
/// each workgroup runs. It does not need to: every slot of the ancestor stack
/// (`trace_stack` in `traversal/tree.wesl`) is written before it is read, from
/// the root down to the level the descent stops at, and a ray restarts no
/// lower than that level. Zeroing the 2.5 KiB stack made the trace 2.6 times
/// slower on Metal (Radeon Pro 5300M: 21.7 against 8.3 ms at 1280x720). The
/// game and the GPU tests both build the pipeline with this value.
pub const ZERO_INIT_WORKGROUP_MEMORY: bool = false;

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
    /// Request capacity, word offset of the request stamps, iteration cap,
    /// beam iteration cap ([`BEAM_OFF`]: no prepass; camera mode only).
    pub limits: [u32; 4],
    /// Tile rows at the bottom of a camera trace that the prepass's dispatch
    /// traces from the camera ([`head_rows`]); y to w unused.
    pub head: [u32; 4],
}

/// The tile rows at the bottom of a `height`-pixel camera trace that the
/// prepass's dispatch traces from the camera while its beams march (decision
/// 0026): `share` of the rows, rounded down, never all of them. The shader
/// clamps the same way (`head_rows` in `view.wesl`).
pub fn head_rows(height: u32, share: f32) -> u32 {
    let (_, rows) = beam_tiles(1, height);
    ((rows as f32 * share.clamp(0.0, 1.0)) as u32).min(rows - 1)
}

/// Bindings: nodes, segments 0 to 3 (read), brick table and feedback
/// (read-write), hit records (read-write), parameters (uniform), beam start
/// distances (read-write). The beam prepass uses the same layout.
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
    v.push(storage_entry(9, false, c));
    v
}

/// Beam tiles over a `width x height` trace: columns and rows.
pub fn beam_tiles(width: u32, height: u32) -> (u32, u32) {
    (
        width.max(1).div_ceil(BEAM_TILE),
        height.max(1).div_ceil(BEAM_TILE),
    )
}

/// The beam prepass's output for a `width x height` trace: one `f32` start
/// distance per tile.
pub fn beam_buffer(device: &Device, width: u32, height: u32) -> Buffer {
    let (columns, rows) = beam_tiles(width, height);
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("primary.beam"),
        size: u64::from(columns) * u64::from(rows) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// The bind group of one view: pools, its hit buffer, its parameters and
/// its beam start distances ([`beam_buffer`]).
pub fn trace_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    pools: &GpuPools,
    hits: &Buffer,
    params: &Buffer,
    beam: &Buffer,
) -> BindGroup {
    let mut buffers: Vec<&Buffer> = vec![&pools.nodes];
    buffers.extend(pools.segments.iter());
    buffers.extend([&pools.table, &pools.feedback, hits, params, beam]);
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

/// Workgroups of the prepass over a `width x height` camera trace whose
/// bottom `head` tile rows ([`head_rows`], the value in `TraceParams::head`)
/// it traces: 8x8 beam tiles over the rows above, then one per head tile.
pub fn beam_groups(width: u32, height: u32, head: u32) -> u32 {
    let (columns, rows) = beam_tiles(width, height);
    let head = head.min(rows - 1);
    columns.div_ceil(BEAM_WORKGROUP) * (rows - head).div_ceil(BEAM_WORKGROUP) + columns * head
}

/// Records the beam prepass over a `width x height` camera trace's tiles,
/// and the trace of its bottom `head` tile rows (the value in
/// `TraceParams::head`). The trace recorded after it reads the start
/// distances when `limits.w` is not [`BEAM_OFF`], and skips the head rows.
pub fn record_beam(
    encoder: &mut CommandEncoder,
    pipeline: &ComputePipeline,
    bind_group: &BindGroup,
    (width, height): (u32, u32),
    head: u32,
    timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
) {
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("primary.beam"),
        timestamp_writes: timestamps,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.dispatch_workgroups(beam_groups(width, height, head), 1, 1);
}
