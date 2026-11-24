// Primary rays: one per pixel, 8x8 workgroups, written as integer hit
// records (layout in world_render's `gbuffer` module). Camera rays start at
// their tile's beam distance when the prepass ran (`beam.wesl`), and miss at
// once where it found the whole tile leaves the tree without stopping (a
// negative distance). In camera mode the bottom `head_rows()` tile rows are
// traced from the camera by the prepass's own dispatch instead, while the
// beam's latency-bound threads would leave the GPU idle (decision 0026).

import package::shaders::common::constants::{HIT_WORDS, HIT_FLAG_HIT, BEAM_TILE};
import package::shaders::traversal::tree::{TraceSettings, Hit, cast_ray_from, trace_miss};
import package::shaders::passes::primary::view::{
    params, beam_starts, camera_dir, beam_columns, beam_rows, beam_enabled, head_rows,
};

@group(0) @binding(7) var<storage, read_write> hits: array<u32>;

@compute @workgroup_size(8, 8, 1)
fn trace(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    // The head rows are the prepass's.
    if gid.y / BEAM_TILE >= beam_rows() - head_rows() {
        return;
    }
    trace_pixel(gid.xy, lane, true);
}

// Traces pixel `pixel`'s ray (a camera ray in camera mode, else the one in
// its hit record), from its tile's beam distance when `use_beam` and the
// prepass ran. `lane` is the local invocation index (below 64): its row of
// the ancestor stack.
fn trace_pixel(pixel: vec2<u32>, lane: u32, use_beam: bool) {
    if pixel.x >= params.size.x || pixel.y >= params.size.y {
        return;
    }
    let i = (pixel.y * params.size.x + pixel.x) * HIT_WORDS;
    var o: vec3<f32>;
    var d: vec3<f32>;
    var t_start = 0.0;
    if params.size.w == 1u {
        o = bitcast<vec3<f32>>(vec3<u32>(hits[i], hits[i + 1u], hits[i + 2u]));
        d = bitcast<vec3<f32>>(vec3<u32>(hits[i + 3u], hits[i + 4u], hits[i + 5u]));
    } else {
        o = params.eye.xyz;
        d = camera_dir(pixel);
        if use_beam && beam_enabled() {
            let tile = pixel / BEAM_TILE;
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
    var h: Hit;
    if t_start < 0.0 {
        h = trace_miss(0u, 0u);
    } else {
        h = cast_ray_from(o, d, s, lane, t_start);
    }
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
