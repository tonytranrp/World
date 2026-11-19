//! World_Testkit: test support with no Bevy dependency.
//!
//! Used as a dev-dependency by the core crates. Nothing here is ever imported
//! by shipping code.
//!
//! - [`profile`]: fast and full test scale, from `WORLD_TEST_PROFILE`
//! - [`policy`]: skip loudly locally, fail in CI when a prerequisite is missing
//! - [`dense`]: the dense reference voxel model, the oracle for the 64-tree,
//!   plus the tree's adapter to it and a canonical-form walk
//! - [`strategies`]: proptest strategies for positions (including world
//!   edges), boxes, single edits, mixed edit-or-fill steps and rays
//! - [`hash`]: xxh3-64 state hashing with a fixed byte order
//! - [`replay`]: deterministic replay records
//! - [`golden`]: golden image and text comparison
//! - [`hits`]: exact comparison of CPU and GPU ray hits

pub mod dense;
pub mod golden;
pub mod hash;
pub mod hits;
pub mod policy;
pub mod profile;
pub mod replay;
pub mod strategies;
