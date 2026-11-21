// The apply pass (decision 0007): writes one frame's upload into the pools.
// Dispatched four times in order: apply_unlink (records made not resident),
// apply_scatter (one workgroup per brick block), apply_table (brick table
// entries), apply_link (every other record). The CPU merges writes to the
// same node or id, so no entry is written twice. Upload layout: world_render
// `brick_cache::stream::batch`.

import package::shaders::common::constants::{
    BLOCK_ALIGN_WORDS, BRICK_REF_OFFSET_MASK, BRICK_REF_SEGMENT_SHIFT,
    BRICK_TABLE_WORDS, UPLOAD_NODE_WRITE_WORDS, UPLOAD_SCATTER_WORDS, UPLOAD_TABLE_WRITE_WORDS,
};
import package::shaders::common::format::Node;

@group(0) @binding(0) var<storage, read_write> nodes: array<Node>;
@group(0) @binding(1) var<storage, read_write> seg0: array<u32>;
@group(0) @binding(2) var<storage, read_write> seg1: array<u32>;
@group(0) @binding(3) var<storage, read_write> seg2: array<u32>;
@group(0) @binding(4) var<storage, read_write> seg3: array<u32>;
@group(0) @binding(5) var<storage, read_write> table: array<u32>;
@group(0) @binding(6) var<storage, read> upload: array<u32>;

const WORKGROUP: u32 = 64u;

// The linear workgroup index of a 2D dispatch grid.
fn group_index(id: vec3<u32>, groups: vec3<u32>) -> u32 {
    return id.y * groups.x + id.x;
}

fn write_segment(segment: u32, word: u32, value: u32) {
    switch segment {
        case 0u: { if word < arrayLength(&seg0) { seg0[word] = value; } }
        case 1u: { if word < arrayLength(&seg1) { seg1[word] = value; } }
        case 2u: { if word < arrayLength(&seg2) { seg2[word] = value; } }
        case 3u: { if word < arrayLength(&seg3) { seg3[word] = value; } }
        default: {}
    }
}

fn write_node(entry_word: u32) {
    let index = upload[entry_word];
    if index < arrayLength(&nodes) {
        nodes[index] = Node(
            upload[entry_word + 1u],
            upload[entry_word + 2u],
            upload[entry_word + 3u],
            upload[entry_word + 4u],
        );
    }
}

@compute @workgroup_size(64, 1, 1)
fn apply_unlink(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let e = group_index(wid, groups) * WORKGROUP + li;
    if e < upload[0] {
        write_node(upload[4] + e * UPLOAD_NODE_WRITE_WORDS);
    }
}

@compute @workgroup_size(64, 1, 1)
fn apply_scatter(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let e = group_index(wid, groups);
    if e >= upload[1] {
        return;
    }
    let at = upload[5] + e * UPLOAD_SCATTER_WORDS;
    let src = upload[8] + upload[at];
    let reference = upload[at + 1u];
    let words = upload[at + 2u];
    let segment = reference >> BRICK_REF_SEGMENT_SHIFT;
    let dst = (reference & BRICK_REF_OFFSET_MASK) * BLOCK_ALIGN_WORDS;
    for (var w = li; w < words; w += WORKGROUP) {
        write_segment(segment, dst + w, upload[src + w]);
    }
}

@compute @workgroup_size(64, 1, 1)
fn apply_table(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let e = group_index(wid, groups) * WORKGROUP + li;
    if e >= upload[3] {
        return;
    }
    let at = upload[7] + e * UPLOAD_TABLE_WRITE_WORDS;
    let slot = upload[at] * BRICK_TABLE_WORDS;
    if slot + 1u < arrayLength(&table) {
        table[slot] = upload[at + 1u];
        table[slot + 1u] = upload[at + 2u];
    }
}

@compute @workgroup_size(64, 1, 1)
fn apply_link(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    let e = group_index(wid, groups) * WORKGROUP + li;
    if e < upload[2] {
        write_node(upload[6] + e * UPLOAD_NODE_WRITE_WORDS);
    }
}
