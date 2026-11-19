//! World_Render: the Bevy render plugin for the micro-voxel world.
//!
//! One public [`WorldRenderPlugin`] and one [`prelude`]; everything else is
//! crate-internal. The capability tier decides what the renderer may use.

pub mod passes;
mod plugin;
pub mod prelude;
pub mod tier;

pub use plugin::WorldRenderPlugin;
