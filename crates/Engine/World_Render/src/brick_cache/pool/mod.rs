//! The fixed GPU pools and their CPU-side slot bookkeeping.
//!
//! - `layout`: sizes from config, profile and adapter limits, fixed at startup
//! - `alloc`: brick space per segment, node child groups and brick ids
//! - `gpu`: the storage buffers themselves

pub mod alloc;
pub mod gpu;
pub mod layout;
