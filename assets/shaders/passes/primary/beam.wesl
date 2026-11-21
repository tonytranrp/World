// The primary pass's beam prepass (decision 0023): one thread per
// BEAM_TILE x BEAM_TILE tile of camera rays finds the distance before which
// none of the tile's rays can stop, and the trace starts its rays there.

import package::shaders::common::constants::BEAM_TILE;
import package::shaders::traversal::beam::{BeamSettings, beam_start};
import package::shaders::passes::primary::view::{params, beam_starts, camera_dir, beam_columns};

@compute @workgroup_size(8, 8, 1)
fn beam(@builtin(global_invocation_id) gid: vec3<u32>) {
    let columns = beam_columns();
    let rows = (params.size.y + BEAM_TILE - 1u) / BEAM_TILE;
    if gid.x >= columns || gid.y >= rows {
        return;
    }
    // The tile's corner pixels; every ray of the tile is a positive
    // combination of theirs (a pinhole camera's directions are affine in
    // the pixel position, up to length).
    let p0 = gid.xy * BEAM_TILE;
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
    beam_starts[gid.y * columns + gid.x] = beam_start(params.eye.xyz, c, spread, s);
}
