//! Composite: voxels, then the sky.
//!
//! `voxel` draws the primary hits into the view's colour and depth right
//! after Bevy's opaque pass, marking covered pixels with alpha 0. The sky is
//! a post-process blended by `strength` times the pixel's alpha, so it fills
//! only what no voxel covered. Both shaders are WESL assets; editing them
//! (or the shared modules they import) hot-reloads the passes.

pub mod voxel;

use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;

/// Asset path of the composite shader.
pub const SKY_SHADER_PATH: &str = "shaders/passes/composite/sky.wesl";

/// Put on a camera to composite the sky over its view.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, ShaderType)]
#[extract_app(RenderApp)]
pub struct SkyComposite {
    /// `0.0` leaves the view unchanged, `1.0` replaces it with sky.
    pub strength: f32,
}

impl Default for SkyComposite {
    fn default() -> Self {
        Self { strength: 1.0 }
    }
}

impl FullscreenMaterial for SkyComposite {
    fn fragment_shader() -> ShaderRef {
        SKY_SHADER_PATH.into()
    }
}

pub(crate) fn build(app: &mut App) {
    app.add_plugins(FullscreenMaterialPlugin::<SkyComposite>::default());
}
