// Lighting v0: voxel hit records to a lit colour, one thread per traced
// pixel, at the trace's (internal) resolution.
// Material colour times a fixed light per face; coarse hits (from the pixel
// footprint or a missing brick) are lit the same way with their cell's
// representative material. The lit target holds the colour in rgb and the
// hit's depth (reverse Z, as Bevy's depth texture stores it) in alpha; a
// miss is alpha -1. The voxel composite stretches it over the viewport.

import package::shaders::common::constants::{HIT_PLANE_WORDS, HIT_FLAG_HIT};

struct ShadeParams {
    // Traced width, height, material count, unused.
    size: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> hits: array<u32>;
@group(0) @binding(1) var<storage, read> colors: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> params: ShadeParams;
@group(0) @binding(3) var lit: texture_storage_2d<rgba32float, write>;

// Light per entry face: -x, +x, -y, +y, -z, +z, inside.
fn face_light(face: u32) -> f32 {
    switch face {
        case 0u: { return 0.72; }
        case 1u: { return 0.80; }
        case 2u: { return 0.45; }
        case 3u: { return 1.00; }
        case 4u: { return 0.62; }
        case 5u: { return 0.88; }
        default: { return 1.0; }
    }
}

@compute @workgroup_size(8, 8, 1)
fn shade(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = id.xy;
    if pixel.x >= params.size.x || pixel.y >= params.size.y {
        return;
    }
    // The G-buffer's shading plane: packed word, depth.
    let i = (pixel.y * params.size.x + pixel.x) * HIT_PLANE_WORDS;
    let packed = hits[i];
    let flags = packed >> 24u;
    if (flags & HIT_FLAG_HIT) == 0u {
        textureStore(lit, pixel, vec4<f32>(0.0, 0.0, 0.0, -1.0));
        return;
    }
    let material = min(packed & 0xFFFFu, max(params.size.z, 1u) - 1u);
    let face = (packed >> 20u) & 0xFu;
    let base = colors[material].rgb;
    let depth = bitcast<f32>(hits[i + 1u]);
    textureStore(lit, pixel, vec4<f32>(base * face_light(face), depth));
}
