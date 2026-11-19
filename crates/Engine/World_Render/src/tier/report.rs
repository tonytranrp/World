use wgpu::{Backend, DeviceType, DownlevelFlags, Features, Limits};

/// Everything the tier logic needs to know about one GPU adapter.
///
/// Plain data, so tests can build one by hand without touching a GPU.
#[derive(Clone, Debug)]
pub struct AdapterReport {
    pub name: String,
    pub backend: Backend,
    pub device_type: DeviceType,
    pub features: Features,
    pub limits: Limits,
    pub downlevel_flags: DownlevelFlags,
}
