// Primary rays: one per pixel, 8x8 workgroups, written as integer hit
// records (layout in world_render's `gbuffer` module). Camera rays start at
// their tile's beam distance when the prepass ran (`beam.wesl`).

import package::shaders::common::constants::{HIT_WORDS, HIT_FLAG_HIT, BEAM_TILE};
import package::shaders::traversal::tree::{TraceSettings, cast_ray_from};
import package::shaders::passes::primary::view::{
    params, beam_starts, camera_dir, beam_columns, beam_enabled,
};

@group(0) @binding(7) var<storage, read_write> hits: array<u32>;

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
    var t_start = 0.0;
    if params.size.w == 1u {
        o = bitcast<vec3<f32>>(vec3<u32>(hits[i], hits[i + 1u], hits[i + 2u]));
        d = bitcast<vec3<f32>>(vec3<u32>(hits[i + 3u], hits[i + 4u], hits[i + 5u]));
    } else {
        o = params.eye.xyz;
        d = camera_dir(gid.xy);
        if beam_enabled() {
            let tile = gid.xy / BEAM_TILE;
            t_start = beam_starts[tile.y * beam_columns() + tile.x];
        }
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
    let h = cast_ray_from(o, d, s, lane, t_start);
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
