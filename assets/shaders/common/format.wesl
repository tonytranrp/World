// The GPU tree format of decision 0007: node records, brick blocks, the
// brick table, the feedback layout, the upload layout and the hit record.
//
// Mirrored by hand from world_render (brick_cache::format, ::feedback,
// ::stream::batch and gbuffer). The test `shader_constants_match_rust`
// parses this file and fails on any difference; Xtask codegen will generate
// it later. Do not change a value here alone.

const NODE_WORDS: u32 = 4u;
const ROOT_NODE: u32 = 0u;
const INFO_MATERIAL_MASK: u32 = 0xFFFFu;
const INFO_KIND_SHIFT: u32 = 16u;
const INFO_KIND_MASK: u32 = 0x3u;
const INFO_EDITED_BIT: u32 = 0x40000u;
const KIND_INTERNAL: u32 = 0u;
const KIND_BRICK: u32 = 1u;
const KIND_NOT_RESIDENT: u32 = 2u;
const KIND_UNIFORM: u32 = 3u;
const BRICK_REF_OFFSET_MASK: u32 = 0xFFFFFFFu;
const BRICK_REF_SEGMENT_SHIFT: u32 = 28u;
const MAX_SEGMENTS: u32 = 4u;
const BLOCK_ALIGN_WORDS: u32 = 4u;
const BLOCK_HEADER_WORDS: u32 = 4u;
const BLOCK_COUNT_MASK: u32 = 0xFFu;
const BLOCK_PALETTE_WORDS_SHIFT: u32 = 8u;
const BLOCK_PALETTE_WORDS_MASK: u32 = 0xFFFFu;
const BLOCK_FLAGS_SHIFT: u32 = 24u;
const BLOCK_FLAG_EDITED: u32 = 1u;
const LEAF_MASK_WORDS: u32 = 2u;
const PAYLOAD_UNIFORM_BIT: u32 = 0x80000000u;
const PALETTE_LEN_MASK: u32 = 0xFFu;
const PALETTE_BITS_SHIFT: u32 = 8u;
const BRICK_TABLE_WORDS: u32 = 2u;
const INVALID_REF: u32 = 0xFFFFFFFFu;

const FEEDBACK_HEADER_WORDS: u32 = 4u;
const REQUEST_WORDS: u32 = 2u;
const VICTIM_WORDS: u32 = 2u;
const REQUEST_LEVEL_SHIFT: u32 = 16u;
const REQUEST_PRIORITY_SHIFT: u32 = 20u;
const REQUEST_PRIORITY_MAX: u32 = 0xFFFu;

const UPLOAD_HEADER_WORDS: u32 = 16u;
const UPLOAD_NODE_WRITE_WORDS: u32 = 5u;
const UPLOAD_SCATTER_WORDS: u32 = 4u;
const UPLOAD_TABLE_WRITE_WORDS: u32 = 4u;

const HIT_WORDS: u32 = 8u;
const HIT_FLAG_HIT: u32 = 1u;
const HIT_FLAG_RESIDENCY: u32 = 2u;
const HIT_FLAG_FOOTPRINT: u32 = 4u;
const HIT_FLAG_EXHAUSTED: u32 = 8u;
const FACE_INSIDE: u32 = 6u;

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
