// GENERATED FILE, DO NOT EDIT. Regenerate with `cargo xtask codegen`.
//
// The constants the shaders share with World_Render, rendered from
// `world_render::internals::shader_constants()` (decision 0013). To change a
// value, change the Rust constant and rerun the command; the test
// `generated_shader_constants_are_current` fails until this file matches.
// Each group is named after the World_Render module that defines it.

// brick_cache::format
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
const BLOCK_HEADER_WORDS: u32 = 7u;
const BLOCK_DEFAULT_WORD: u32 = 2u;
const BLOCK_DEFAULT_SHIFT: u32 = 16u;
const BLOCK_DEFAULT_MASK_WORD: u32 = 3u;
const BLOCK_FULL_MASK_WORD: u32 = 5u;
const BLOCK_COUNT_MASK: u32 = 0xFFu;
const BLOCK_PALETTE_WORDS_SHIFT: u32 = 8u;
const BLOCK_PALETTE_WORDS_MASK: u32 = 0xFFFFu;
const BLOCK_FLAGS_SHIFT: u32 = 24u;
const BLOCK_FLAG_EDITED: u32 = 1u;
const LEAF_MASK_WORDS: u32 = 2u;
const PAYLOAD_UNIFORM_BIT: u32 = 0x80000000u;
const PALETTE_LEN_MASK: u32 = 0xFFu;
const PALETTE_BITS_SHIFT: u32 = 8u;
const PALETTE_PAIR_SHIFT: u32 = 16u;
const BRICK_TABLE_WORDS: u32 = 2u;
const INVALID_REF: u32 = 0xFFFFFFFFu;

// brick_cache::feedback
const FEEDBACK_HEADER_WORDS: u32 = 4u;
const REQUEST_WORDS: u32 = 2u;
const VICTIM_WORDS: u32 = 2u;
const REQUEST_LEVEL_SHIFT: u32 = 16u;
const REQUEST_PRIORITY_SHIFT: u32 = 20u;
const REQUEST_PRIORITY_MAX: u32 = 0xFFFu;

// brick_cache::stream::batch
const UPLOAD_HEADER_WORDS: u32 = 16u;
const UPLOAD_NODE_WRITE_WORDS: u32 = 5u;
const UPLOAD_SCATTER_WORDS: u32 = 4u;
const UPLOAD_TABLE_WRITE_WORDS: u32 = 4u;

// gbuffer
const HIT_WORDS: u32 = 8u;
const HIT_FLAG_HIT: u32 = 1u;
const HIT_FLAG_RESIDENCY: u32 = 2u;
const HIT_FLAG_FOOTPRINT: u32 = 4u;
const HIT_FLAG_EXHAUSTED: u32 = 8u;
const FACE_INSIDE: u32 = 6u;
