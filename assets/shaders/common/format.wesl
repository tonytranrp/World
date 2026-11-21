// The GPU tree format of decision 0007: the node record and its accessors.
//
// The format's constants (node info bits, brick block, brick table,
// feedback, upload and hit record layouts) are generated from World_Render
// into `constants.wesl` by `cargo xtask codegen` (decision 0013); import them
// from there. This file is written by hand.

import package::shaders::common::constants::{
    INFO_MATERIAL_MASK, INFO_KIND_SHIFT, INFO_KIND_MASK,
};

// One node record (16 bytes).
struct Node {
    mask_lo: u32,
    mask_hi: u32,
    reference: u32,
    info: u32,
}

fn node_kind(n: Node) -> u32 {
    return (n.info >> INFO_KIND_SHIFT) & INFO_KIND_MASK;
}

fn node_material(n: Node) -> u32 {
    return n.info & INFO_MATERIAL_MASK;
}

// Whether bit `slot` (0 to 63) of a two-word mask is set.
fn mask_bit(lo: u32, hi: u32, slot: u32) -> bool {
    if slot < 32u {
        return ((lo >> slot) & 1u) != 0u;
    }
    return ((hi >> (slot - 32u)) & 1u) != 0u;
}

// Set bits of a two-word mask below `slot` (0 to 63): a child's rank.
fn mask_rank(lo: u32, hi: u32, slot: u32) -> u32 {
    if slot < 32u {
        return countOneBits(lo & ((1u << slot) - 1u));
    }
    return countOneBits(lo) + countOneBits(hi & ((1u << (slot - 32u)) - 1u));
}

// The child slot (0 to 63) a local coordinate falls in at `level`.
fn child_slot(local: vec3<u32>, level: u32) -> u32 {
    let c = (local >> vec3<u32>(2u * level)) & vec3<u32>(3u);
    return c.x | (c.y << 2u) | (c.z << 4u);
}
