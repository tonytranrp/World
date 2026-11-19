// The procedural sky gradient, shared by every pass that needs the sky colour.

const HORIZON: vec3<f32> = vec3<f32>(0.78, 0.86, 0.95);
const ZENITH: vec3<f32> = vec3<f32>(0.22, 0.45, 0.85);

// `height` is 0 at the horizon and 1 straight up. Returns linear RGB.
fn sky_color(height: f32) -> vec3<f32> {
    let t = smoothstep(0.0, 1.0, clamp(height, 0.0, 1.0));
    return mix(HORIZON, ZENITH, t);
}
