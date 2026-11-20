//! The GPU side of the pools: storage buffers, created once from a layout.

use wgpu::util::DeviceExt;
use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device};

use super::layout::PoolLayout;
use crate::brick_cache::format::{BRICK_TABLE_WORDS, MAX_SEGMENTS, NODE_BYTES};

/// Size of the stand-in bound to segment slots the layout does not use.
const DUMMY_SEGMENT_BYTES: u64 = 256;

/// Every buffer the brick cache owns on the GPU.
pub struct GpuPools {
    /// Node records, `layout.node_count` of them.
    pub nodes: Buffer,
    /// Exactly [`MAX_SEGMENTS`] buffers; slots past the layout's segment
    /// count hold a small zeroed stand-in so bind groups stay complete.
    pub segments: Vec<Buffer>,
    /// The brick table: block ref and last-used frame per brick id. Zeroed
    /// at creation; a last-used frame of 0 marks a free id.
    pub table: Buffer,
    /// Request header, requests, then one request stamp per node.
    pub feedback: Buffer,
    /// Victim header and victims, written by the eviction pass.
    pub victims: Buffer,
    pub layout: PoolLayout,
}

fn storage(device: &Device, label: &str, size: u64, extra: BufferUsages) -> Buffer {
    device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size: size.max(16).next_multiple_of(16),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST | BufferUsages::COPY_SRC | extra,
        mapped_at_creation: false,
    })
}

impl GpuPools {
    /// Creates the buffers. wgpu zero-initializes them, which is a valid
    /// empty state: the root record is an internal node with no children,
    /// every brick id is free and no node has been requested.
    pub fn new(device: &Device, layout: PoolLayout) -> Self {
        let nodes = storage(
            device,
            "brick_cache.nodes",
            u64::from(layout.node_count) * NODE_BYTES,
            BufferUsages::empty(),
        );
        let segments = (0..MAX_SEGMENTS as usize)
            .map(|i| match layout.segments.get(i) {
                Some(&bytes) => storage(
                    device,
                    &format!("brick_cache.segment{i}"),
                    bytes,
                    BufferUsages::empty(),
                ),
                None => device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("brick_cache.segment_unused"),
                    contents: &[0; DUMMY_SEGMENT_BYTES as usize],
                    usage: BufferUsages::STORAGE,
                }),
            })
            .collect();
        let table = storage(
            device,
            "brick_cache.table",
            u64::from(layout.brick_ids) * u64::from(BRICK_TABLE_WORDS) * 4,
            BufferUsages::empty(),
        );
        let feedback = storage(
            device,
            "brick_cache.feedback",
            layout.feedback_words() * 4,
            BufferUsages::empty(),
        );
        let victims = storage(
            device,
            "brick_cache.victims",
            u64::from(layout.victim_block_words()) * 4,
            BufferUsages::empty(),
        );
        Self {
            nodes,
            segments,
            table,
            feedback,
            victims,
            layout,
        }
    }
}
