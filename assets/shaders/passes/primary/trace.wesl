// Primary rays: one per pixel, 8x8 workgroups, written as integer hit
// records (layout in world_render's `gbuffer` module).

import package::shaders::common::format::{HIT_WORDS, HIT_FLAG_HIT};
import package::shaders::traversal::tree::{TraceSettings, cast_ray};

struct TraceParams {
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    // Camera position in voxels.
    eye: vec4<f32>,
    // Tree origin; w is the root level.
    origin: vec4<i32>,
    // Width, height, frame, mode (0 camera rays, 1 rays read from `hits`).
    size: vec4<u32>,
    // Pixel angle, footprint bias, voxels per metre, unused.
    lod: vec4<f32>,
    // Request capacity, stamps offset, iteration cap, unused.
    limits: vec4<u32>,
}

@group(0) @binding(7) var<storage, read_write> hits: array<u32>;
@group(0) @binding(8) var<uniform> params: TraceParams;

fn camera_dir(pixel: vec2<u32>) -> vec3<f32> {
    let size = vec2<f32>(params.size.xy);
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / size;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // Reverse Z: depth 1 is the near plane; 0.5 is twice as far.
    let near = params.world_from_clip * vec4<f32>(ndc, 1.0, 1.0);
    let far = params.world_from_clip * vec4<f32>(ndc, 0.5, 1.0);
    return normalize(far.xyz / far.w - near.xyz / near.w);
}

@compute @workgroup_size(8, 8, 1)
fn trace(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    if gid.x >= params.size.x || gid.y >= params.size.y {
        return;
    }
    let i = (gid.y * params.size.x + gid.x) * HIT_WORDS;
    var o: vec3<f32>;
    var d: vec3<f32>;
    if params.size.w == 1u {
        o = bitcast<vec3<f32>>(vec3<u32>(hits[i], hits[i + 1u], hits[i + 2u]));
        d = bitcast<vec3<f32>>(vec3<u32>(hits[i + 3u], hits[i + 4u], hits[i + 5u]));
    } else {
        o = params.eye.xyz;
        d = camera_dir(gid.xy);
    }
    let s = TraceSettings(
        params.origin.xyz,
        u32(params.origin.w),
        params.size.z,
        params.lod.x,
        params.lod.y,
        params.limits.x,
        params.limits.y,
        params.limits.z,
    );
    let h = cast_ray(o, d, s, lane);
    var depth = 0.0;
    if (h.flags & HIT_FLAG_HIT) != 0u {
        let p = (o + d * h.t) / params.lod.z;
        let clip = params.clip_from_world * vec4<f32>(p, 1.0);
        // A hit at the eye (a ray starting inside a voxel) is as near as it gets.
        depth = select(1.0, clamp(clip.z / clip.w, 0.0, 1.0), clip.w > 1e-6);
    }
    hits[i] = bitcast<u32>(h.voxel.x);
    hits[i + 1u] = bitcast<u32>(h.voxel.y);
    hits[i + 2u] = bitcast<u32>(h.voxel.z);
    hits[i + 3u] = (h.material & 0xFFFFu) | (h.level << 16u) | (h.face << 20u) | (h.flags << 24u);
    hits[i + 4u] = bitcast<u32>(h.t);
    hits[i + 5u] = bitcast<u32>(depth);
    hits[i + 6u] = h.steps;
    hits[i + 7u] = 0u;
}
