//! Pool sizes and brick segments, from config, profile and adapter limits.

use crate::brick_cache::config::{BrickCacheConfig, PoolSizes};
use crate::brick_cache::format::{BRICK_REF_OFFSET_MASK, MAX_SEGMENTS, NODE_BYTES};

const MIB: u64 = 1024 * 1024;

/// Segment size when the adapter reports no usable limit.
pub const FALLBACK_SEGMENT_BYTES: u64 = 256 * MIB;
/// The largest segment a 28-bit offset in 16-byte units can address (4 GiB).
pub const MAX_ADDRESSABLE_SEGMENT_BYTES: u64 = (BRICK_REF_OFFSET_MASK as u64 + 1) * 16;
/// Segments and the node pool are whole multiples of this.
const GRANULE: u64 = 256;
/// Words of the feedback header: request count, overflow, two reserved.
pub const FEEDBACK_HEADER_WORDS: u32 = 4;
/// Words per request: `x | y << 16`, `z | level << 16 | priority << 20`.
pub const REQUEST_WORDS: u32 = 2;
/// Words per victim: brick id, the block ref it was chosen with.
pub const VICTIM_WORDS: u32 = 2;

/// The buffer limits a pool must respect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferLimits {
    pub max_storage_buffer_binding_size: u64,
    pub max_buffer_size: u64,
}

impl BufferLimits {
    pub fn from_wgpu(limits: &wgpu::Limits) -> Self {
        Self {
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_buffer_size: limits.max_buffer_size,
        }
    }

    /// The largest buffer that can be bound whole as storage.
    pub fn bindable(&self) -> u64 {
        self.max_storage_buffer_binding_size
            .min(self.max_buffer_size)
    }
}

/// The size of one brick segment: the adapter's bindable size, capped by
/// `cap` (0 for none) and by what a brick reference can address, rounded
/// down to 256 bytes. `FALLBACK_SEGMENT_BYTES` when the adapter reports 0.
pub fn segment_bytes(limits: BufferLimits, cap: u64) -> u64 {
    let reported = limits.bindable();
    let mut size = if reported == 0 {
        FALLBACK_SEGMENT_BYTES
    } else {
        reported
    };
    if cap > 0 {
        size = size.min(cap);
    }
    size = size.min(MAX_ADDRESSABLE_SEGMENT_BYTES);
    (size / GRANULE * GRANULE).max(GRANULE)
}

/// Every size the GPU pools and the CPU bookkeeping are built with. Fixed
/// for the process lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolLayout {
    /// Node records in the node pool.
    pub node_count: u32,
    /// Bytes of each brick segment; at most [`MAX_SEGMENTS`].
    pub segments: Vec<u64>,
    /// Entries in the brick table.
    pub brick_ids: u32,
    /// Allocations each segment's allocator can track.
    pub allocs_per_segment: u32,
    /// Child groups the node allocator can track.
    pub node_groups: u32,
    pub upload_budget: u64,
    pub request_capacity: u32,
    pub victim_capacity: u32,
}

impl PoolLayout {
    /// The layout for `sizes` (chosen by profile) under `limits`.
    pub fn plan(sizes: PoolSizes, config: &BrickCacheConfig, limits: BufferLimits) -> Self {
        let seg = segment_bytes(limits, u64::from(config.max_segment_mib) * MIB);
        let pool = sizes
            .brick_pool_bytes()
            .min(seg * u64::from(MAX_SEGMENTS))
            .max(GRANULE);
        let mut segments = Vec::new();
        let mut left = pool;
        while left > 0 {
            let s = left.min(seg);
            segments.push((s / GRANULE * GRANULE).max(GRANULE));
            left -= s;
        }

        // The request stamps (one word per node) share the feedback buffer
        // with the requests, so both must fit one binding.
        let feedback_fixed =
            u64::from(FEEDBACK_HEADER_WORDS + config.request_capacity * REQUEST_WORDS) * 4;
        let bindable = if limits.bindable() == 0 {
            FALLBACK_SEGMENT_BYTES
        } else {
            limits.bindable()
        };
        let node_bytes = sizes
            .node_pool_bytes()
            .min(bindable)
            .min((bindable.saturating_sub(feedback_fixed)) * NODE_BYTES / 4);
        let node_count = (node_bytes / NODE_BYTES).clamp(64, u64::from(u32::MAX / 2)) as u32;

        let mean = u64::from(config.mean_brick_bytes.max(64));
        let brick_ids = (pool / mean)
            .clamp(1024, bindable / 8)
            .min(u64::from(u32::MAX / 2)) as u32;
        let allocs_per_segment = (seg / mean).clamp(1024, 1 << 26) as u32;
        let node_groups = (u64::from(node_count) / 4).clamp(64, 1 << 26) as u32;

        Self {
            node_count,
            segments,
            brick_ids,
            allocs_per_segment,
            node_groups,
            upload_budget: sizes.upload_budget_bytes().min(pool),
            request_capacity: config.request_capacity,
            victim_capacity: config.victim_capacity,
        }
    }

    pub fn brick_pool_bytes(&self) -> u64 {
        self.segments.iter().sum()
    }

    pub fn node_pool_bytes(&self) -> u64 {
        u64::from(self.node_count) * NODE_BYTES
    }

    /// Words of the feedback buffer: header, requests, then one request
    /// stamp per node.
    pub fn feedback_words(&self) -> u64 {
        u64::from(FEEDBACK_HEADER_WORDS)
            + u64::from(self.request_capacity) * u64::from(REQUEST_WORDS)
            + u64::from(self.node_count)
    }

    /// Word offset of the first request stamp in the feedback buffer.
    pub fn stamps_offset(&self) -> u32 {
        FEEDBACK_HEADER_WORDS + self.request_capacity * REQUEST_WORDS
    }

    /// Words of the readable prefix of the feedback buffer: header and requests.
    pub fn request_block_words(&self) -> u32 {
        FEEDBACK_HEADER_WORDS + self.request_capacity * REQUEST_WORDS
    }

    /// Words of the victim block: header and victims.
    pub fn victim_block_words(&self) -> u32 {
        FEEDBACK_HEADER_WORDS + self.victim_capacity * VICTIM_WORDS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(binding: u64, buffer: u64) -> BufferLimits {
        BufferLimits {
            max_storage_buffer_binding_size: binding,
            max_buffer_size: buffer,
        }
    }

    #[test]
    fn segments_follow_the_binding_limit() {
        // DX12 and Vulkan clamp bindings to 2^31 - 1 bytes.
        let dx = limits((1 << 31) - 1, 1 << 40);
        assert_eq!(segment_bytes(dx, 0), (1 << 31) - 256);
        // Metal often allows one big segment, capped by what a ref addresses.
        let metal = limits(1 << 40, 1 << 40);
        assert_eq!(segment_bytes(metal, 0), MAX_ADDRESSABLE_SEGMENT_BYTES);
        // The config caps it.
        assert_eq!(segment_bytes(dx, 64 * MIB), 64 * MIB);
        // Nothing reported: the documented fallback.
        assert_eq!(segment_bytes(limits(0, 0), 0), FALLBACK_SEGMENT_BYTES);
    }

    #[test]
    fn the_pc_pool_splits_into_two_segments_on_dx12() {
        let config = BrickCacheConfig::default();
        let layout = PoolLayout::plan(config.pc, &config, limits((1 << 31) - 1, (1 << 31) - 1));
        assert_eq!(layout.segments.len(), 2);
        assert_eq!(layout.brick_pool_bytes(), 2048 * MIB);
        assert_eq!(layout.node_pool_bytes(), 128 * MIB);
        assert!(layout.feedback_words() * 4 <= (1 << 31) - 1);
    }

    #[test]
    fn the_pool_is_capped_at_the_segments_the_shaders_bind() {
        let config = BrickCacheConfig::default();
        let layout = PoolLayout::plan(config.pc, &config, limits(128 * MIB, 128 * MIB));
        assert_eq!(layout.segments.len(), MAX_SEGMENTS as usize);
        assert_eq!(layout.brick_pool_bytes(), 512 * MIB);
        // 128 MiB binding: the node pool shares it with nothing, but the
        // stamps (a quarter of its size) plus requests must fit the feedback binding.
        assert!(layout.feedback_words() * 4 <= 128 * MIB);
    }

    #[test]
    fn the_floor_mac_gets_one_segment_when_metal_allows_it() {
        let config = BrickCacheConfig::default();
        let layout = PoolLayout::plan(config.floor, &config, limits(1 << 32, 1 << 32));
        assert_eq!(layout.segments, vec![768 * MIB]);
    }
}
