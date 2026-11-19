//! Offscreen rendering for tests.
//!
//! Renders into an image instead of a window, on the adapter the test asks for,
//! and returns the pixels. Golden images are stable only on the software
//! adapter, so that is the default; `WORLD_TEST_ADAPTER=gpu` uses the real GPU.

use std::sync::{Arc, Mutex};

use bevy::app::{App, SubApps};
use bevy::asset::{Assets, RenderAssetUsages};
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::ecs::world::World;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::render_resource::{PollType, TextureFormat};
use bevy::render::renderer::RenderDevice;
use bevy::render::settings::{Backends, PowerPreference, WgpuSettings};
use bevy::render::view::Msaa;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use wgpu::{Instance, InstanceDescriptor, RequestAdapterOptions};
use world_testkit::golden::Rgba8Image;

/// Environment variable choosing the adapter: `software` (default) or `gpu`.
pub const ADAPTER_ENV: &str = "WORLD_TEST_ADAPTER";

/// Frames to let pass before a screenshot, so pipelines are compiled and ready.
const WARMUP_FRAMES: usize = 3;
/// Frames to wait for the screenshot read-back before giving up.
const CAPTURE_FRAMES: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestAdapter {
    /// WARP on Windows, lavapipe on Linux. Identical output on every machine.
    Software,
    /// The machine's real GPU. Output differs between GPUs.
    Gpu,
}

impl TestAdapter {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim) {
            None | Some("") | Some("software") => Ok(Self::Software),
            Some("gpu") => Ok(Self::Gpu),
            Some(other) => Err(format!(
                "{ADAPTER_ENV}={other:?} is not an adapter; use \"software\" or \"gpu\""
            )),
        }
    }

    /// Panics on an unknown value so a typo cannot silently pick the wrong adapter.
    pub fn from_env() -> Self {
        Self::parse(std::env::var(ADAPTER_ENV).ok().as_deref()).unwrap_or_else(|e| panic!("{e}"))
    }

    fn backends(self) -> Backends {
        match self {
            Self::Gpu => Backends::PRIMARY,
            Self::Software if cfg!(target_os = "windows") => Backends::DX12,
            Self::Software if cfg!(target_os = "macos") => Backends::METAL,
            Self::Software => Backends::VULKAN,
        }
    }

    fn force_fallback(self) -> bool {
        self == Self::Software
    }

    fn settings(self) -> WgpuSettings {
        WgpuSettings {
            backends: Some(self.backends()),
            force_fallback_adapter: self.force_fallback(),
            power_preference: PowerPreference::HighPerformance,
            ..default()
        }
    }

    /// Whether an adapter of this kind exists on this machine.
    pub fn is_available(self) -> bool {
        let mut descriptor = InstanceDescriptor::new_without_display_handle();
        descriptor.backends = self.backends();
        let instance = Instance::new(descriptor);
        let options = RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: self.force_fallback(),
            compatible_surface: None,
            ..Default::default()
        };
        bevy::tasks::block_on(instance.request_adapter(&options)).is_ok()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderError {
    NoAdapter(TestAdapter),
    /// The screenshot never arrived.
    CaptureTimedOut,
    UnsupportedFormat(String),
}

/// An app that renders into an image rather than a window.
pub struct OffscreenApp {
    apps: SubApps,
    target: Handle<Image>,
}

impl OffscreenApp {
    /// Builds the app on `adapter`. `configure` adds plugins and systems before
    /// the app is finished. Fails with [`RenderError::NoAdapter`] when the
    /// adapter does not exist, which tests turn into a skip or a failure with
    /// `world_testkit::policy::skip_or_fail`.
    pub fn new(
        adapter: TestAdapter,
        width: u32,
        height: u32,
        configure: impl FnOnce(&mut App),
    ) -> Result<Self, RenderError> {
        if !adapter.is_available() {
            return Err(RenderError::NoAdapter(adapter));
        }

        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(RenderPlugin {
                    render_creation: adapter.settings().into(),
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                // No window system, and no global log subscriber that a second app would clash with.
                .disable::<WinitPlugin>()
                .disable::<LogPlugin>(),
        );
        configure(&mut app);
        app.finish();
        app.cleanup();
        let mut apps = std::mem::take(app.sub_apps_mut());

        let mut image =
            Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
        image.asset_usage = RenderAssetUsages::RENDER_WORLD;
        let target = apps
            .main
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(image);
        Ok(Self { apps, target })
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.apps.main.world_mut()
    }

    /// A camera that renders into the offscreen image with Bevy's own tone
    /// mapping, dithering and anti-aliasing off, so colours come out exact.
    pub fn spawn_camera(&mut self, clear: Color) -> Entity {
        let target = RenderTarget::from(self.target.clone());
        self.world_mut()
            .spawn((
                Camera3d::default(),
                target,
                Camera {
                    clear_color: ClearColorConfig::Custom(clear),
                    ..default()
                },
                Tonemapping::None,
                DebandDither::Disabled,
                Msaa::Off,
            ))
            .id()
    }

    /// Runs one frame and waits for the GPU to finish it.
    pub fn update(&mut self) {
        self.apps.update();
        if let Some(device) = self.apps.main.world().get_resource::<RenderDevice>() {
            let _ = device.wgpu_device().poll(PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        }
    }

    /// Renders a few frames, then reads back the target image.
    pub fn capture(&mut self) -> Result<Rgba8Image, RenderError> {
        for _ in 0..WARMUP_FRAMES {
            self.update();
        }
        let slot: Arc<Mutex<Option<Image>>> = Arc::default();
        let sink = slot.clone();
        let target = self.target.clone();
        self.world_mut().spawn(Screenshot::image(target)).observe(
            move |captured: On<ScreenshotCaptured>| {
                *sink.lock().unwrap() = Some(captured.image.clone());
            },
        );
        for _ in 0..CAPTURE_FRAMES {
            self.update();
            if let Some(image) = slot.lock().unwrap().take() {
                return to_rgba8(image);
            }
        }
        Err(RenderError::CaptureTimedOut)
    }
}

fn to_rgba8(image: Image) -> Result<Rgba8Image, RenderError> {
    let (width, height) = (image.width(), image.height());
    let format = image.texture_descriptor.format;
    let mut data = image
        .data
        .ok_or_else(|| RenderError::UnsupportedFormat("image has no CPU data".into()))?;
    match format {
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm => {}
        TextureFormat::Bgra8UnormSrgb | TextureFormat::Bgra8Unorm => {
            for px in data.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        other => return Err(RenderError::UnsupportedFormat(format!("{other:?}"))),
    }
    Ok(Rgba8Image::new(width, height, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_is_the_default() {
        assert_eq!(TestAdapter::parse(None), Ok(TestAdapter::Software));
        assert_eq!(TestAdapter::parse(Some("")), Ok(TestAdapter::Software));
    }

    #[test]
    fn parses_the_real_gpu_choice() {
        assert_eq!(TestAdapter::parse(Some("gpu")), Ok(TestAdapter::Gpu));
    }

    #[test]
    fn unknown_adapter_names_the_variable() {
        let err = TestAdapter::parse(Some("cpu")).unwrap_err();
        assert!(err.contains(ADAPTER_ENV) && err.contains("cpu"));
    }

    #[test]
    fn bgra_is_swapped_to_rgba() {
        let mut image = Image::new_target_texture(1, 1, TextureFormat::Bgra8UnormSrgb, None);
        image.data = Some(vec![10, 20, 30, 255]);
        assert_eq!(to_rgba8(image).unwrap().pixel(0, 0), [30, 20, 10, 255]);
    }

    #[test]
    fn unsupported_format_is_an_error() {
        let mut image = Image::new_target_texture(1, 1, TextureFormat::Rgba16Float, None);
        image.data = Some(vec![0; 8]);
        assert!(matches!(
            to_rgba8(image),
            Err(RenderError::UnsupportedFormat(_))
        ));
    }
}
