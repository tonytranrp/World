// Composite v2: the lit voxel image to colour and depth.
// Lighting writes the lit target at the trace's (internal) resolution:
// colour in rgb, depth (reverse Z) in alpha, alpha -1 for a miss. Each
// screen pixel reads the traced pixel under it, so this pass does one load
// per screen pixel and no shading. Covered pixels get alpha 0, which tells
// the sky composite to leave them alone. Misses are discarded.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct CompositeParams {
    // Traced width, height, unused, unused.
    size: vec4<u32>,
    // Viewport width, height, unused, unused.
    viewport: vec4<u32>,
}

@group(0) @binding(0) var lit: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: CompositeParams;

struct CompositeOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> CompositeOutput {
    let screen = vec2<u32>(in.position.xy);
    if screen.x >= params.viewport.x || screen.y >= params.viewport.y {
        discard;
    }
    let pixel = min(screen * params.size.xy / params.viewport.xy, params.size.xy - 1u);
    let texel = textureLoad(lit, pixel, 0);
    if texel.a < 0.0 {
        discard;
    }
    var out: CompositeOutput;
    out.color = vec4<f32>(texel.rgb, 0.0);
    out.depth = texel.a;
    return out;
}
