//! Recording one frame's upload: the serialized batch goes through a
//! `StagingBelt` into the upload buffer, then the apply pass (unlink,
//! scatter, table, link) writes it into the pools.

use std::num::NonZeroU64;

use wgpu::util::StagingBelt;
use wgpu::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutEntry, BindingType, Buffer,
    BufferBindingType, BufferDescriptor, BufferUsages, CommandEncoder, ComputePassDescriptor,
    ComputePipeline, Device, ShaderStages,
};

use super::batch::BatchCounts;
use crate::brick_cache::format::MAX_SEGMENTS;
use crate::brick_cache::pool::gpu::GpuPools;

/// Threads per apply workgroup.
pub const APPLY_WORKGROUP: u32 = 64;
/// Largest workgroup count per dimension wgpu guarantees.
const MAX_GROUPS_PER_DIM: u32 = 65535;

/// Entry points of `brick_cache/apply.wesl`, in dispatch order.
pub const APPLY_ENTRY_POINTS: [&str; 4] =
    ["apply_unlink", "apply_scatter", "apply_table", "apply_link"];

/// A 2D workgroup grid covering `groups` workgroups; shaders linearize it
/// as `id.y * num_workgroups.x + id.x`.
pub fn grid(groups: u32) -> (u32, u32) {
    if groups <= MAX_GROUPS_PER_DIM {
        (groups, 1)
    } else {
        (MAX_GROUPS_PER_DIM, groups.div_ceil(MAX_GROUPS_PER_DIM))
    }
}

pub(crate) fn storage_entry(
    binding: u32,
    read_only: bool,
    stages: ShaderStages,
) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: stages,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Bindings of the apply pass: nodes, segments 0 to 3, brick table (all
/// read-write), upload buffer (read).
pub fn apply_layout_entries() -> Vec<BindGroupLayoutEntry> {
    let mut v: Vec<_> = (0..2 + MAX_SEGMENTS)
        .map(|b| storage_entry(b, false, ShaderStages::COMPUTE))
        .collect();
    v.push(storage_entry(2 + MAX_SEGMENTS, true, ShaderStages::COMPUTE));
    v
}

/// The upload buffer. It grows when a batch needs more room (it is staging,
/// not a pool); `generation` changes then so bind groups are rebuilt.
pub struct UploadBuffer {
    pub buffer: Buffer,
    pub generation: u64,
}

impl UploadBuffer {
    pub fn new(device: &Device, bytes: u64) -> Self {
        Self {
            buffer: Self::create(device, bytes),
            generation: 0,
        }
    }

    fn create(device: &Device, bytes: u64) -> Buffer {
        device.create_buffer(&BufferDescriptor {
            label: Some("brick_cache.upload"),
            size: bytes.max(256).next_multiple_of(256),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Makes room for `bytes`, growing to the next power of two.
    pub fn reserve(&mut self, device: &Device, bytes: u64) {
        if bytes > self.buffer.size() {
            self.buffer = Self::create(device, bytes.next_power_of_two());
            self.generation += 1;
        }
    }
}

/// The apply pass's bind group over the pools and the upload buffer.
pub fn apply_bind_group(
    device: &Device,
    layout: &BindGroupLayout,
    pools: &GpuPools,
    upload: &UploadBuffer,
) -> BindGroup {
    let mut buffers: Vec<&Buffer> = vec![&pools.nodes];
    buffers.extend(pools.segments.iter());
    buffers.push(&pools.table);
    buffers.push(&upload.buffer);
    let entries: Vec<BindGroupEntry> = buffers
        .iter()
        .enumerate()
        .map(|(i, b)| BindGroupEntry {
            binding: i as u32,
            resource: b.as_entire_binding(),
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("brick_cache.apply"),
        layout,
        entries: &entries,
    })
}

/// The four apply pipelines, in [`APPLY_ENTRY_POINTS`] order.
pub struct ApplyPipelines<'a> {
    pub unlink: &'a ComputePipeline,
    pub scatter: &'a ComputePipeline,
    pub table: &'a ComputePipeline,
    pub link: &'a ComputePipeline,
}

/// Copies the serialized batch into the upload buffer through the belt and
/// records the apply pass. The caller has reserved room (`UploadBuffer::reserve`)
/// and built `bind_group` for the current upload buffer; after recording
/// everything into `encoder` it calls `belt.finish_and_recall_on_submit`.
pub fn record_apply(
    encoder: &mut CommandEncoder,
    belt: &mut StagingBelt,
    upload: &UploadBuffer,
    words: &[u32],
    counts: BatchCounts,
    bind_group: &BindGroup,
    pipelines: &ApplyPipelines<'_>,
) {
    if stage_upload(encoder, belt, upload, words) {
        record_apply_pass(encoder, counts, bind_group, pipelines, None);
    }
}

/// Copies the serialized batch into the upload buffer through the belt.
/// Returns false, recording nothing, for an empty batch.
pub fn stage_upload(
    encoder: &mut CommandEncoder,
    belt: &mut StagingBelt,
    upload: &UploadBuffer,
    words: &[u32],
) -> bool {
    let bytes: &[u8] = bytemuck::cast_slice(words);
    let Some(size) = NonZeroU64::new(bytes.len() as u64) else {
        return false;
    };
    let mut view = belt.write_buffer(encoder, &upload.buffer, 0, size);
    view.copy_from_slice(bytes);
    true
}

/// Records the apply pass (unlink, scatter, table, link) over an upload
/// staged by [`stage_upload`], with optional timestamps.
pub fn record_apply_pass(
    encoder: &mut CommandEncoder,
    counts: BatchCounts,
    bind_group: &BindGroup,
    pipelines: &ApplyPipelines<'_>,
    timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
) {
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("brick_cache.apply"),
        timestamp_writes: timestamps,
    });
    pass.set_bind_group(0, bind_group, &[]);
    let per_thread = |n: u32| n.div_ceil(APPLY_WORKGROUP);
    let dispatches = [
        (pipelines.unlink, per_thread(counts.unlink)),
        (pipelines.scatter, counts.scatter),
        (pipelines.table, per_thread(counts.table)),
        (pipelines.link, per_thread(counts.link)),
    ];
    for (pipeline, groups) in dispatches {
        if groups == 0 {
            continue;
        }
        let (x, y) = grid(groups);
        pass.set_pipeline(pipeline);
        pass.dispatch_workgroups(x, y, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_cover_large_counts() {
        assert_eq!(grid(10), (10, 1));
        let (x, y) = grid(200_000);
        assert!(x * y >= 200_000 && x <= MAX_GROUPS_PER_DIM);
    }
}
