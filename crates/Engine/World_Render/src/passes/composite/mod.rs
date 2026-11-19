//! Composite, version 0: the sky.
//!
//! The voxel passes do not exist yet, so there is no G-buffer depth to tell
//! sky from surface. Until they land, composite paints the procedural sky over
//! the view, blended by `strength`. The shader is a WESL asset that imports a
//! shared sky module, so editing either file hot-reloads the pass.

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
