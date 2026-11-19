//! World_Game: gameplay for the destruction sandbox.
//!
//! One public [`WorldGamePlugin`] and one [`prelude`]; each domain folder
//! contributes its own sub-plugin.

mod player;
mod plugin;
pub mod prelude;

pub use plugin::WorldGamePlugin;
