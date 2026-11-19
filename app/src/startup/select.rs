//! Picks the adapter the game will run on. A pure function over probe results.

use wgpu::{Backend, DeviceType};
use world_render::prelude::{AdapterReport, Classification, Profile, Unsupported, classify};

pub struct Chosen {
    pub report: AdapterReport,
    pub classification: Classification,
}

/// An adapter that was found but cannot run the renderer, and why.
pub struct Rejection {
    pub name: String,
    pub backend: Backend,
    pub reason: Unsupported,
}

fn device_rank(kind: DeviceType) -> u8 {
    match kind {
        DeviceType::DiscreteGpu => 4,
        DeviceType::IntegratedGpu => 3,
        DeviceType::VirtualGpu => 2,
        DeviceType::Other => 1,
        DeviceType::Cpu => 0,
    }
}

/// Native API first: Metal on macOS, DX12 over Vulkan on Windows.
fn backend_rank(backend: Backend) -> u8 {
    match backend {
        Backend::Metal => 3,
        Backend::Dx12 => 2,
        Backend::Vulkan => 1,
        _ => 0,
    }
}

/// Best supported adapter, ranked by device type, then backend, then tier.
/// Fails with every rejection reason when nothing can run the renderer.
pub fn choose(reports: &[AdapterReport], profile: Profile) -> Result<Chosen, Vec<Rejection>> {
    let mut best: Option<((u8, u8, u8), Chosen)> = None;
    let mut rejections = Vec::new();

    for report in reports {
        match classify(report) {
            Ok(classification) => {
                let classification = classification.with_profile(profile);
                let key = (
                    device_rank(report.device_type),
                    backend_rank(report.backend),
                    classification.tier as u8,
                );
                if best.as_ref().is_none_or(|(k, _)| key > *k) {
                    best = Some((
                        key,
                        Chosen {
                            report: report.clone(),
                            classification,
                        },
                    ));
                }
            }
            Err(reason) => rejections.push(Rejection {
                name: report.name.clone(),
                backend: report.backend,
                reason,
            }),
        }
    }

    best.map(|(_, chosen)| chosen).ok_or(rejections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{DownlevelFlags, Features, Limits};

    fn adapter(name: &str, backend: Backend, device_type: DeviceType) -> AdapterReport {
        AdapterReport {
            name: name.into(),
            backend,
            device_type,
            features: Features::empty(),
            limits: Limits::default(),
            downlevel_flags: DownlevelFlags::all(),
        }
    }

    #[test]
    fn discrete_beats_integrated() {
        let reports = [
            adapter("igpu", Backend::Dx12, DeviceType::IntegratedGpu),
            adapter("dgpu", Backend::Vulkan, DeviceType::DiscreteGpu),
        ];
        let chosen = choose(&reports, Profile::Full).ok().unwrap();
        assert_eq!(chosen.report.name, "dgpu");
    }

    #[test]
    fn dx12_beats_vulkan_on_the_same_gpu() {
        let reports = [
            adapter("gpu", Backend::Vulkan, DeviceType::DiscreteGpu),
            adapter("gpu", Backend::Dx12, DeviceType::DiscreteGpu),
        ];
        let chosen = choose(&reports, Profile::Full).ok().unwrap();
        assert_eq!(chosen.report.backend, Backend::Dx12);
    }

    #[test]
    fn software_adapter_is_last_resort() {
        let reports = [
            adapter("warp", Backend::Dx12, DeviceType::Cpu),
            adapter("igpu", Backend::Vulkan, DeviceType::IntegratedGpu),
        ];
        let chosen = choose(&reports, Profile::Full).ok().unwrap();
        assert_eq!(chosen.report.name, "igpu");
    }

    #[test]
    fn unsupported_adapters_are_reported_when_none_qualify() {
        let reports = [adapter("old", Backend::Gl, DeviceType::DiscreteGpu)];
        let rejections = choose(&reports, Profile::Full).err().unwrap();
        assert_eq!(rejections.len(), 1);
        assert_eq!(rejections[0].reason, Unsupported::Backend(Backend::Gl));
    }

    #[test]
    fn no_adapters_means_no_rejections_either() {
        assert!(choose(&[], Profile::Full).err().unwrap().is_empty());
    }
}
