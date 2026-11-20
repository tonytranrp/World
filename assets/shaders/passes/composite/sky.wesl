// Composite v0: paints the procedural sky over the view.
// Saving this file (or the sky module it imports) hot-reloads the pass.
// Pixels the voxel composite covered carry alpha 0 and keep their colour;
// everything else (alpha 1 from the clear) gets the sky.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import package::shaders::common::sky::sky_color;

struct SkyComposite {
    strength: f32,
}

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> settings: SkyComposite;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let scene = textureSample(screen_texture, screen_sampler, in.uv);
    // uv.y is 0 at the top of the screen and 1 at the bottom.
    let sky = sky_color(1.0 - in.uv.y);
    return vec4<f32>(mix(scene.rgb, sky, settings.strength * scene.a), 1.0);
}
