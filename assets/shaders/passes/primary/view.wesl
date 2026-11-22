// What the primary pass's two entry points share: the view parameters, the
// beam's per-tile start distances and camera ray generation. The hit
// records are the trace's own (binding 7).

import package::shaders::common::constants::BEAM_TILE;

struct TraceParams {
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    // Camera position in voxels.
    eye: vec4<f32>,
    // Tree origin; w is the root level.
    origin: vec4<i32>,
    // Width, height, frame, mode (0 camera rays written as the G-buffer's
    // two planes, 1 rays read from `hits`, 2 camera rays written as full
    // records).
    size: vec4<u32>,
    // Pixel angle, footprint bias, voxels per metre, unused.
    lod: vec4<f32>,
    // Request capacity, stamps offset, iteration cap, beam iteration cap
    // (0: no beam prepass).
    limits: vec4<u32>,
    // Tile rows at the bottom of a camera trace that the prepass's dispatch
    // traces from the camera (decision 0026); y to w unused.
    head: vec4<u32>,
}

@group(0) @binding(8) var<uniform> params: TraceParams;
// One start distance per BEAM_TILE x BEAM_TILE tile of camera rays, in
// voxels along each ray, row-major (decision 0023); negative where every
// ray of the tile leaves the tree without stopping (decision 0026).
@group(0) @binding(9) var<storage, read_write> beam_starts: array<f32>;

fn camera_dir(pixel: vec2<u32>) -> vec3<f32> {
    let size = vec2<f32>(params.size.xy);
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / size;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // Reverse Z: depth 1 is the near plane; 0.5 is twice as far.
    let near = params.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let far = params.world_from_clip * vec4<f32>(ndc, 0.5, 1.0);
    return normalize(far.xyz / far.w - near.xyz / near.w);
}

// Beam tiles across the traced width.
fn beam_columns() -> u32 {
    return (params.size.x + BEAM_TILE - 1u) / BEAM_TILE;
}

// Beam tile rows across the traced height.
fn beam_rows() -> u32 {
    return (params.size.y + BEAM_TILE - 1u) / BEAM_TILE;
}

// Tile rows at the bottom traced from the camera by the prepass's dispatch:
// camera mode only, and never every row.
fn head_rows() -> u32 {
    return select(0u, min(params.head.x, beam_rows() - 1u), camera_mode());
}

// Whether rays come from the camera (not from the hit buffer).
fn camera_mode() -> bool {
    return params.size.w != 1u;
}

// Whether camera rays start at their tile's beam distance.
fn beam_enabled() -> bool {
    return camera_mode() && params.limits.w != 0u;
}
