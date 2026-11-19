use bevy::math::UVec2;
use bevy::prelude::Resource;
use wgpu::Backend;

use super::classify::{Capabilities, Tier};
use super::report::AdapterReport;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Budget profile. A profile changes limits only, never code paths, so a PC
/// run in `MacFloor` exercises the same code as a real baseline Mac.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    Full,
    /// Approximates the 2019 16-inch MacBook Pro: tier B, 720p internal,
    /// a 1 GiB brick pool and four worker threads.
    MacFloor,
}

/// Derived budgets, before user settings override them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budgets {
    pub internal_resolution: UVec2,
    pub brick_pool_bytes: u64,
    /// `None` leaves the thread count to Bevy.
    pub worker_threads: Option<usize>,
}

impl Budgets {
    pub fn for_profile(profile: Profile, adapter: &AdapterReport) -> Self {
        let (internal_resolution, pool_cap, worker_threads) = match profile {
            Profile::Full => (UVec2::new(2560, 1440), 2 * GIB, None),
            Profile::MacFloor => (UVec2::new(1280, 720), GIB, Some(4)),
        };
        Self {
            internal_resolution,
            brick_pool_bytes: pool_cap.min(adapter.limits.max_buffer_size),
            worker_threads,
        }
    }
}

/// The decision, shared with every render module as a resource.
#[derive(Resource, Clone, Debug)]
pub struct RenderTier {
    pub tier: Tier,
    pub capabilities: Capabilities,
    pub profile: Profile,
    pub budgets: Budgets,
    pub backend: Backend,
    pub adapter_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{DeviceType, DownlevelFlags, Features, Limits};

    fn adapter(max_buffer_size: u64) -> AdapterReport {
        let mut limits = Limits::default();
        limits.max_buffer_size = max_buffer_size;
        AdapterReport {
            name: "test".into(),
            backend: Backend::Vulkan,
            device_type: DeviceType::DiscreteGpu,
            features: Features::empty(),
            limits,
            downlevel_flags: DownlevelFlags::all(),
        }
    }

    #[test]
    fn mac_floor_is_720p_one_gib_four_threads() {
        let b = Budgets::for_profile(Profile::MacFloor, &adapter(u64::MAX));
        assert_eq!(b.internal_resolution, UVec2::new(1280, 720));
        assert_eq!(b.brick_pool_bytes, GIB);
        assert_eq!(b.worker_threads, Some(4));
    }

    #[test]
    fn full_profile_is_1440p_two_gib() {
        let b = Budgets::for_profile(Profile::Full, &adapter(u64::MAX));
        assert_eq!(b.internal_resolution, UVec2::new(2560, 1440));
        assert_eq!(b.brick_pool_bytes, 2 * GIB);
        assert_eq!(b.worker_threads, None);
    }

    #[test]
    fn pool_never_exceeds_the_adapter_buffer_limit() {
        let b = Budgets::for_profile(Profile::Full, &adapter(512 * MIB));
        assert_eq!(b.brick_pool_bytes, 512 * MIB);
    }
}
