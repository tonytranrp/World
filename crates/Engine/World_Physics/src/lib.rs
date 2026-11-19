//! World_Physics: the Bevy physics plugin.
//!
//! The physics engine is named only inside the `rapier` module, so swapping
//! it for another engine touches this crate alone.

mod plugin;
pub mod prelude;
mod rapier;

pub use plugin::WorldPhysicsPlugin;
