//! World_Game: gameplay for the destruction sandbox.
//!
//! One public [`WorldGamePlugin`] and one [`prelude`]; each domain folder
//! contributes its own sub-plugin.

mod interaction;
mod player;
mod plugin;
pub mod prelude;
mod residency;
mod ui;

pub use plugin::WorldGamePlugin;
