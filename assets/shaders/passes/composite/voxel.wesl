// Composite v1: voxel hit records to colour and depth.
// Material colour times a fixed light per face; coarse hits (from the pixel
// footprint or a missing brick) are drawn the same way with their cell's
// representative material. Covered pixels get alpha 0, which tells the sky
// composite to leave them alone. Misses are discarded.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import package::shaders::common::format::{HIT_WORDS, HIT_FLAG_HIT};

struct CompositeParams {
    // Width, height, material count, unused.
    size: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> hits: array<u32>;
@group(0) @binding(1) var<storage, read> colors: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> params: CompositeParams;

struct CompositeOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

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

@fragment
fn fragment(in: FullscreenVertexOutput) -> CompositeOutput {
    let pixel = vec2<u32>(in.position.xy);
    if pixel.x >= params.size.x || pixel.y >= params.size.y {
        discard;
    }
    let i = (pixel.y * params.size.x + pixel.x) * HIT_WORDS;
    let packed = hits[i + 3u];
    let flags = packed >> 24u;
    if (flags & HIT_FLAG_HIT) == 0u {
        discard;
    }
    let material = min(packed & 0xFFFFu, max(params.size.z, 1u) - 1u);
    let face = (packed >> 20u) & 0xFu;
    let base = colors[material].rgb;
    var out: CompositeOutput;
    out.color = vec4<f32>(base * face_light(face), 0.0);
    out.depth = bitcast<f32>(hits[i + 5u]);
    return out;
}
