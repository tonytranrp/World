// The eviction pass (decision 0007): one 256-thread workgroup builds an age
// histogram of the brick table in workgroup memory, picks the youngest age
// at least `min_age` with `wanted` bricks at that age or older, and
// compacts those bricks' ids and block refs into the victim block. Free ids
// (last-used frame 0) are skipped. Data never moves here.

import package::shaders::common::format::{
    BRICK_TABLE_WORDS, INVALID_REF, FEEDBACK_HEADER_WORDS, VICTIM_WORDS,
};

struct EvictParams {
    frame: u32,
    min_age: u32,
    wanted: u32,
    id_count: u32,
    capacity: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

@group(0) @binding(0) var<storage, read> table: array<u32>;
@group(0) @binding(1) var<storage, read_write> victims: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> params: EvictParams;

const BINS: u32 = 256u;

var<workgroup> histogram: array<atomic<u32>, 256>;
var<workgroup> threshold: u32;

// The age bin of brick `id`, or BINS for a free id.
fn age_bin(id: u32) -> u32 {
    let block = table[id * BRICK_TABLE_WORDS];
    let last = table[id * BRICK_TABLE_WORDS + 1u];
    if last == 0u || block == INVALID_REF {
        return BINS;
    }
    return min(params.frame - last, BINS - 1u);
}

@compute @workgroup_size(256, 1, 1)
fn evict(@builtin(local_invocation_index) li: u32) {
    atomicStore(&histogram[li], 0u);
    workgroupBarrier();
    let ids = min(params.id_count, arrayLength(&table) / BRICK_TABLE_WORDS);
    for (var id = li; id < ids; id += BINS) {
        let bin = age_bin(id);
        if bin < BINS {
            atomicAdd(&histogram[bin], 1u);
        }
    }
    workgroupBarrier();
    if li == 0u {
        let min_age = i32(min(params.min_age, BINS - 1u));
        var chosen = u32(min_age);
        var acc = 0u;
        for (var t = i32(BINS) - 1; t >= min_age; t--) {
            acc += atomicLoad(&histogram[u32(t)]);
            if acc >= params.wanted {
                chosen = u32(t);
                break;
            }
        }
        threshold = chosen;
    }
    let th = workgroupUniformLoad(&threshold);
    for (var id = li; id < ids; id += BINS) {
        let bin = age_bin(id);
        if bin < BINS && bin >= th {
            let slot = atomicAdd(&victims[0], 1u);
            if slot < params.capacity {
                let at = FEEDBACK_HEADER_WORDS + slot * VICTIM_WORDS;
                atomicStore(&victims[at], id);
                atomicStore(&victims[at + 1u], table[id * BRICK_TABLE_WORDS]);
            } else {
                atomicAdd(&victims[1], 1u);
            }
        }
    }
}
