//! The only import the rest of the workspace needs from World_Render.

pub use crate::WorldRenderPlugin;
pub use crate::brick_cache::config::{BrickCacheConfig, Stage};
pub use crate::brick_cache::stream::feed::VoxelWorldFeed;
pub use crate::passes::composite::{SKY_SHADER_PATH, SkyComposite};
pub use crate::tier::{
    AdapterReport, Budgets, Capabilities, Classification, Profile, RenderTier, Tier, Unsupported,
    classify, probe_adapters,
};
