use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;

use world_core::material::MaterialTable;
use world_reload::AppConfigExt;

use crate::passes;
use crate::tier::RenderTier;

/// Registers everything the renderer needs. The tier is decided before the GPU
/// device exists (see `tier`), so the app hands the decision in here.
pub struct WorldRenderPlugin {
    tier: RenderTier,
}

impl WorldRenderPlugin {
    pub fn new(tier: RenderTier) -> Self {
        Self { tier }
    }
}

impl Plugin for WorldRenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.tier.clone())
            .add_config::<MaterialTable>()
            .add_systems(Startup, log_active_adapter);
        passes::build(app);
    }
}

/// Logs the tier we decided on and the adapter Bevy actually created, so a
/// mismatch between the two is visible in the very first lines of output.
fn log_active_adapter(tier: Res<RenderTier>, active: Option<Res<RenderAdapterInfo>>) {
    info!(
        "render tier {:?} | backend {:?} | adapter \"{}\" | profile {:?} | internal {}x{} | brick pool {} MiB",
        tier.tier,
        tier.backend,
        tier.adapter_name,
        tier.profile,
        tier.budgets.internal_resolution.x,
        tier.budgets.internal_resolution.y,
        tier.budgets.brick_pool_bytes / (1024 * 1024),
    );
    match active {
        Some(info) => {
            let matches = info.backend == tier.backend && info.name == tier.adapter_name;
            if matches {
                info!("device created on the selected adapter");
            } else {
                warn!(
                    "device adapter differs from selection: got {:?} \"{}\"",
                    info.backend, info.name
                );
            }
        }
        None => warn!("adapter info not available yet at startup"),
    }
}
