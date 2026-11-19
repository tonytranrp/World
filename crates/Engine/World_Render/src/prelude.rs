//! The only import the rest of the workspace needs from World_Render.

pub use crate::WorldRenderPlugin;
pub use crate::tier::{
    AdapterReport, Budgets, Capabilities, Classification, Profile, RenderTier, Tier, Unsupported,
    classify, probe_adapters,
};
