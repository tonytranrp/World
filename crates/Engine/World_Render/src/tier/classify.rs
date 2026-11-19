use std::fmt;

use wgpu::{Backend, DownlevelFlags, Features};

use super::profile::Profile;
use super::report::AdapterReport;

/// Capability tier. Ordered so that `Tier::A > Tier::B > Tier::C`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Unsupported: no compute shaders. The world renderer cannot run.
    C,
    /// Baseline: compute shaders and storage buffers. Fully functional.
    B,
    /// Baseline plus hardware extras (ray queries and mesh shaders).
    A,
}

/// Individual capability flags other modules query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub compute: bool,
    pub ray_query: bool,
    pub mesh_shader: bool,
}

/// Why an adapter cannot run the world renderer at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unsupported {
    /// The OpenGL backend has no compute shaders worth the name here.
    Backend(Backend),
    /// The adapter reports no compute shader support.
    NoCompute,
    /// A limit is below what the traversal passes need.
    LimitTooLow {
        name: &'static str,
        have: u64,
        need: u64,
    },
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(b) => write!(f, "the {b:?} backend cannot run the voxel renderer"),
            Self::NoCompute => write!(f, "no compute shader support"),
            Self::LimitTooLow { name, have, need } => {
                write!(f, "{name} is {have}, but at least {need} is required")
            }
        }
    }
}

/// The verdict for one adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Classification {
    pub tier: Tier,
    pub capabilities: Capabilities,
}

impl Classification {
    /// Applies a profile. `MacFloor` forces tier B and hides the tier A extras,
    /// so a PC exercises exactly the code a baseline Mac would.
    pub fn with_profile(mut self, profile: Profile) -> Self {
        if profile == Profile::MacFloor {
            self.tier = self.tier.min(Tier::B);
            self.capabilities.ray_query = false;
            self.capabilities.mesh_shader = false;
        }
        self
    }
}

const MIB: u64 = 1024 * 1024;

/// Minimum limits the compute traversal passes rely on (WebGPU defaults).
const MIN_COMPUTE_INVOCATIONS_PER_WORKGROUP: u32 = 256;
const MIN_STORAGE_BUFFERS_PER_STAGE: u32 = 8;
const MIN_STORAGE_TEXTURES_PER_STAGE: u32 = 4;
const MIN_STORAGE_BUFFER_BINDING_BYTES: u64 = 128 * MIB;
const MIN_BIND_GROUPS: u32 = 4;

/// Maps adapter data to a tier. Capability comes from features and limits only,
/// never from vendor or operating system names.
pub fn classify(report: &AdapterReport) -> Result<Classification, Unsupported> {
    if report.backend == Backend::Gl {
        return Err(Unsupported::Backend(report.backend));
    }
    if !report
        .downlevel_flags
        .contains(DownlevelFlags::COMPUTE_SHADERS)
    {
        return Err(Unsupported::NoCompute);
    }

    let l = &report.limits;
    let checks: [(&'static str, u64, u64); 5] = [
        (
            "max_compute_invocations_per_workgroup",
            l.max_compute_invocations_per_workgroup.into(),
            MIN_COMPUTE_INVOCATIONS_PER_WORKGROUP.into(),
        ),
        (
            "max_storage_buffers_per_shader_stage",
            l.max_storage_buffers_per_shader_stage.into(),
            MIN_STORAGE_BUFFERS_PER_STAGE.into(),
        ),
        (
            "max_storage_textures_per_shader_stage",
            l.max_storage_textures_per_shader_stage.into(),
            MIN_STORAGE_TEXTURES_PER_STAGE.into(),
        ),
        (
            "max_storage_buffer_binding_size",
            l.max_storage_buffer_binding_size,
            MIN_STORAGE_BUFFER_BINDING_BYTES,
        ),
        (
            "max_bind_groups",
            l.max_bind_groups.into(),
            MIN_BIND_GROUPS.into(),
        ),
    ];
    for (name, have, need) in checks {
        if have < need {
            return Err(Unsupported::LimitTooLow { name, have, need });
        }
    }

    let capabilities = Capabilities {
        compute: true,
        ray_query: report.features.contains(Features::EXPERIMENTAL_RAY_QUERY),
        mesh_shader: report.features.contains(Features::EXPERIMENTAL_MESH_SHADER),
    };
    let tier = if capabilities.ray_query && capabilities.mesh_shader {
        Tier::A
    } else {
        Tier::B
    };
    Ok(Classification { tier, capabilities })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{DeviceType, Limits};

    fn report(backend: Backend, features: Features) -> AdapterReport {
        AdapterReport {
            name: "test".into(),
            backend,
            device_type: DeviceType::DiscreteGpu,
            features,
            limits: Limits::default(),
            downlevel_flags: DownlevelFlags::all(),
        }
    }

    #[test]
    fn plain_compute_adapter_is_tier_b() {
        let c = classify(&report(Backend::Vulkan, Features::empty())).unwrap();
        assert_eq!(c.tier, Tier::B);
        assert!(c.capabilities.compute && !c.capabilities.ray_query);
    }

    #[test]
    fn ray_query_and_mesh_shaders_make_tier_a() {
        let f = Features::EXPERIMENTAL_RAY_QUERY | Features::EXPERIMENTAL_MESH_SHADER;
        assert_eq!(classify(&report(Backend::Dx12, f)).unwrap().tier, Tier::A);
    }

    #[test]
    fn ray_query_alone_stays_tier_b() {
        let r = report(Backend::Dx12, Features::EXPERIMENTAL_RAY_QUERY);
        let c = classify(&r).unwrap();
        assert_eq!(c.tier, Tier::B);
        assert!(c.capabilities.ray_query);
    }

    #[test]
    fn gl_backend_is_unsupported() {
        let err = classify(&report(Backend::Gl, Features::empty())).unwrap_err();
        assert_eq!(err, Unsupported::Backend(Backend::Gl));
    }

    #[test]
    fn missing_compute_is_unsupported() {
        let mut r = report(Backend::Vulkan, Features::empty());
        r.downlevel_flags = DownlevelFlags::empty();
        assert_eq!(classify(&r).unwrap_err(), Unsupported::NoCompute);
    }

    #[test]
    fn low_limit_is_reported_by_name() {
        let mut r = report(Backend::Metal, Features::empty());
        r.limits.max_storage_buffers_per_shader_stage = 4;
        match classify(&r).unwrap_err() {
            Unsupported::LimitTooLow { name, have, need } => {
                assert_eq!(name, "max_storage_buffers_per_shader_stage");
                assert_eq!((have, need), (4, 8));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn mac_floor_clamps_tier_a_to_b_and_hides_extras() {
        let f = Features::EXPERIMENTAL_RAY_QUERY | Features::EXPERIMENTAL_MESH_SHADER;
        let c = classify(&report(Backend::Vulkan, f))
            .unwrap()
            .with_profile(Profile::MacFloor);
        assert_eq!(c.tier, Tier::B);
        assert!(!c.capabilities.ray_query && !c.capabilities.mesh_shader);
    }

    #[test]
    fn full_profile_leaves_the_verdict_alone() {
        let f = Features::EXPERIMENTAL_RAY_QUERY | Features::EXPERIMENTAL_MESH_SHADER;
        let c = classify(&report(Backend::Vulkan, f)).unwrap();
        assert_eq!(c, c.with_profile(Profile::Full));
    }
}
