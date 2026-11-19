//! Capability tiers: what the current machine can render.
//!
//! Detection is a pure function of adapter data (`classify`), kept separate
//! from the act of reading adapters (`probe`) so it is trivially testable.

mod classify;
mod probe;
mod profile;
mod report;

pub use classify::{Capabilities, Classification, Tier, Unsupported, classify};
pub use probe::probe_adapters;
pub use profile::{Budgets, Profile, RenderTier};
pub use report::AdapterReport;
