// The node and brick pools as every tracing pass binds them (group 0,
// bindings 0 to 6), and the accessors traversal uses. A pass that imports
// the traversal adds its own bindings from 7 up.

import package::shaders::common::constants::{
    BLOCK_ALIGN_WORDS, BRICK_REF_OFFSET_MASK, BRICK_REF_SEGMENT_SHIFT,
};
import package::shaders::common::format::Node;

@group(0) @binding(0) var<storage, read> pool_nodes: array<Node>;
@group(0) @binding(1) var<storage, read> pool_seg0: array<u32>;
@group(0) @binding(2) var<storage, read> pool_seg1: array<u32>;
@group(0) @binding(3) var<storage, read> pool_seg2: array<u32>;
@group(0) @binding(4) var<storage, read> pool_seg3: array<u32>;
// Block ref and last-used frame per brick id; rays stamp the frame.
@group(0) @binding(5) var<storage, read_write> pool_table: array<u32>;
// Request header, requests, then one request stamp per node.
@group(0) @binding(6) var<storage, read_write> pool_feedback: array<atomic<u32>>;

fn load_node(index: u32) -> Node {
    return pool_nodes[index];
}

// The segment of a brick ref.
fn block_segment(reference: u32) -> u32 {
    return reference >> BRICK_REF_SEGMENT_SHIFT;
}

// The word index of a brick block's start inside its segment.
fn block_base(reference: u32) -> u32 {
    return (reference & BRICK_REF_OFFSET_MASK) * BLOCK_ALIGN_WORDS;
}

// One word of a brick segment. Segments are separate bindings because
// binding arrays of buffers are Vulkan-only.
fn brick_word(segment: u32, word: u32) -> u32 {
    switch segment {
        case 0u: { return pool_seg0[word]; }
        case 1u: { return pool_seg1[word]; }
        case 2u: { return pool_seg2[word]; }
        case 3u: { return pool_seg3[word]; }
        default: { return 0u; }
    }
}
