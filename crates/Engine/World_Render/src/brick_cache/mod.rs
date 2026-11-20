//! The ray-guided brick cache of decision 0007.
//!
//! - `format`: the GPU node and brick block layout, the encoder and a CPU decoder
//! - `pool`: pool sizes, slot bookkeeping and the GPU buffers
//! - `stream`: the feed, the residency table, stage 1 and 2 selection, the
//!   upload batch and the apply pass
//! - `feedback`: requests and victims read back from the GPU
//! - `evict`: the age-histogram eviction pass
//! - `config`: the tuning file
//! - `render`: the per-frame lifecycle in Bevy's render world

pub mod config;
pub mod evict;
pub mod feedback;
pub mod format;
pub mod pool;
pub mod render;
pub mod stream;
