// The primary pass's beam prepass (decision 0023): one thread per
// BEAM_TILE x BEAM_TILE tile of camera rays finds the distance before which
// none of the tile's rays can stop, and the trace starts its rays there.
//
// The march is latency-bound and one thread per tile leaves most of the GPU
// idle, so the same dispatch also traces the bottom `head_rows()` tile rows
// from the camera (decision 0026): workgroups `0..beam_groups()` march
// beams, 8x8 tiles each, over the rows above the head; each later workgroup
// traces one head tile, as the trace pass would without a beam.

import package::shaders::common::constants::BEAM_TILE;
import package::shaders::traversal::beam::{BeamSettings, beam_start};
import package::shaders::passes::primary::view::{
    params, beam_starts, camera_dir, beam_columns, beam_rows, head_rows,
};
import package::shaders::passes::primary::trace::trace_pixel;

// Beam workgroups across the beam's columns.
fn beam_group_columns() -> u32 {
    return (beam_columns() + 7u) / 8u;
}

// Workgroups that march beams: the rows above the head, 8x8 tiles each.
fn beam_groups() -> u32 {
    return beam_group_columns() * ((beam_rows() - head_rows() + 7u) / 8u);
}

@compute @workgroup_size(8, 8, 1)
fn beam(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let columns = beam_columns();
    let head_start = beam_rows() - head_rows();
    let n = beam_groups();
    if group.x >= n {
        // One head tile, from the camera.
        let h = group.x - n;
        let tile = vec2<u32>(h % columns, head_start + h / columns);
        if lane == 0u {
            beam_starts[tile.y * columns + tile.x] = 0.0;
        }
        trace_pixel(tile * BEAM_TILE + local.xy, lane, false);
        return;
    }
    let gx = group.x % beam_group_columns();
    let gy = group.x / beam_group_columns();
    let tile = vec2<u32>(gx, gy) * 8u + local.xy;
    if tile.x >= columns || tile.y >= head_start {
        return;
    }
    // The tile's corner pixels; every ray of the tile is a positive
    // combination of theirs (a pinhole camera's directions are affine in
    // the pixel position, up to length).
    let p0 = tile * BEAM_TILE;
    let p1 = min(p0 + vec2<u32>(BEAM_TILE - 1u), params.size.xy - vec2<u32>(1u));
    let d00 = camera_dir(p0);
    let d10 = camera_dir(vec2<u32>(p1.x, p0.y));
    let d01 = camera_dir(vec2<u32>(p0.x, p1.y));
    let d11 = camera_dir(p1);
    let c = normalize(d00 + d10 + d01 + d11);
    let spread = max(
        max(length(d00 - c), length(d10 - c)),
        max(length(d01 - c), length(d11 - c)),
    );
    let s = BeamSettings(
        params.origin.xyz,
        u32(params.origin.w),
        params.lod.x * params.lod.y,
        params.limits.w,
    );
    beam_starts[tile.y * columns + tile.x] = beam_start(params.eye.xyz, c, spread, s);
}
