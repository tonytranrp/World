//! World_Render: the Bevy render plugin for the micro-voxel world.
//!
//! One public [`WorldRenderPlugin`] and one [`prelude`]; everything else is
//! crate-internal. The capability tier decides what the renderer may use.
//!
//! [`internals`] re-exports the GPU format, pools, streaming and pass
//! recording for this crate's integration tests and benchmarks. It is not
//! an API: nothing outside World_Render's own tests may use it.

mod brick_cache;
mod debug;
mod gbuffer;
mod gpu_timing;
mod graph;
mod passes;
mod plugin;
pub mod prelude;
pub mod tier;

pub use plugin::WorldRenderPlugin;

/// Test and benchmark access to the renderer's internals. Not an API.
#[doc(hidden)]
pub mod internals {
    pub use crate::brick_cache::config::{BrickCacheConfig, LodTuning, PoolSizes, Stage};
    pub use crate::brick_cache::evict::{self, EvictParams};
    pub use crate::brick_cache::feedback::{self, Request, Victim};
    pub use crate::brick_cache::format::{self, GpuNode};
    pub use crate::brick_cache::pool::{alloc, gpu::GpuPools, layout};
    pub use crate::brick_cache::stream::{
        batch, feed, frame, key, residency, select, summaries, upload,
    };
    pub use crate::debug::scene;
    pub mod gpu_timing {
        pub use crate::gpu_timing::*;
    }
    pub mod gbuffer {
        pub use crate::gbuffer::*;
    }
    pub use crate::passes::primary::trace;

    /// Every Rust constant the shaders use, grouped by the module that
    /// defines it, in the order `cargo xtask codegen` writes them to
    /// `assets/shaders/common/constants.wesl` (decision 0013).
    pub fn shader_constants() -> [(&'static str, &'static [(&'static str, u32)]); 5] {
        [
            (
                "brick_cache::format",
                crate::brick_cache::format::SHADER_CONSTANTS,
            ),
            (
                "brick_cache::feedback",
                crate::brick_cache::feedback::SHADER_CONSTANTS,
            ),
            (
                "brick_cache::stream::batch",
                crate::brick_cache::stream::batch::SHADER_CONSTANTS,
            ),
            ("gbuffer", crate::gbuffer::SHADER_CONSTANTS),
            (
                "passes::primary::trace",
                crate::passes::primary::trace::SHADER_CONSTANTS,
            ),
        ]
    }
}
