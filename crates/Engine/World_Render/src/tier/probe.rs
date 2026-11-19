use wgpu::{Backends, Instance, InstanceDescriptor};

use super::report::AdapterReport;

/// Reads every adapter the given backends can see, without creating a device.
///
/// Uses a short-lived wgpu instance of its own, so it can run before Bevy
/// creates the real device. That is what lets the tier decide which render
/// plugins register, and which adapter Bevy is told to use.
pub fn probe_adapters(backends: Backends) -> Vec<AdapterReport> {
    let mut descriptor = InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    let instance = Instance::new(descriptor);

    bevy::tasks::block_on(instance.enumerate_adapters(backends))
        .iter()
        .map(|adapter| {
            let info = adapter.get_info();
            AdapterReport {
                name: info.name,
                backend: info.backend,
                device_type: info.device_type,
                features: adapter.features(),
                limits: adapter.limits(),
                downlevel_flags: adapter.get_downlevel_capabilities().flags,
            }
        })
        .collect()
}
