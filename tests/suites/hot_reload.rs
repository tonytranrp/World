//! The hot-reload engine across crates: the configs we ship, the plugins that
//! register them, the game's reloadable camera, and shader hot reload on the
//! GPU. `gpu_` tests render on the software adapter by default.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bevy::core_pipeline::fullscreen_material::FullscreenMaterialPlugin;
use bevy::prelude::*;
use wgpu::{Backend, DeviceType, DownlevelFlags, Features, Limits};
use world_core::material::MaterialTable;
use world_game::prelude::{PlayerCamera, WorldGamePlugin};
use world_physics::prelude::{PHYSICS_CONFIG_PATH, PhysicsTuning, WorldPhysicsPlugin};
use world_reload::{CodeReloaded, ConfigOrigin, ConfigRegistry, WorldReloadPlugin, parse_config};
use world_render::prelude::{
    AdapterReport, Budgets, Profile, RenderTier, SKY_SHADER_PATH, SkyComposite, WorldRenderPlugin,
    classify,
};
use world_testkit::golden::{Rgba8Image, Tolerance, assert_image_golden};
use world_testkit::policy::skip_or_fail;
use world_testkit_bevy::app::HeadlessApp;
use world_testkit_bevy::assets::DiskAssets;
use world_testkit_bevy::render::{OffscreenApp, RenderError, TestAdapter};
use world_testkit_bevy::schedule::for_each_seed;

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets")
}

fn shipped(path: &str) -> String {
    std::fs::read_to_string(assets_dir().join(path))
        .unwrap_or_else(|e| panic!("assets/{path}: {e}"))
}

fn render_tier() -> RenderTier {
    let report = AdapterReport {
        name: "test adapter".into(),
        backend: Backend::Vulkan,
        device_type: DeviceType::DiscreteGpu,
        features: Features::empty(),
        limits: Limits::default(),
        downlevel_flags: DownlevelFlags::all(),
    };
    let verdict = classify(&report).unwrap();
    RenderTier {
        tier: verdict.tier,
        capabilities: verdict.capabilities,
        profile: Profile::Full,
        budgets: Budgets::for_profile(Profile::Full, &report),
        backend: report.backend,
        adapter_name: report.name,
    }
}

// ---------------------------------------------------------------- configs --

#[test]
fn the_shipped_physics_tuning_is_valid_and_matches_the_defaults() {
    let tuning: PhysicsTuning =
        parse_config(&shipped(PHYSICS_CONFIG_PATH)).unwrap_or_else(|e| panic!("physics.ron: {e}"));
    // The file documents the defaults; if they drift apart one of them is wrong.
    assert_eq!(tuning, PhysicsTuning::default());
}

#[test]
fn the_shipped_material_table_is_valid() {
    let table: MaterialTable =
        parse_config(&shipped("config/materials.ron")).unwrap_or_else(|e| panic!("{e}"));
    assert!(!table.is_empty());
}

/// Every file in `assets/config/` is registered by some plugin and loads
/// cleanly from the real asset folder. An orphan file is a forgotten
/// registration; a failed load is a broken shipped file.
#[test]
fn every_shipped_config_is_registered_and_loads_cleanly() {
    let mut app = HeadlessApp::new();
    app.add_plugins((
        bevy::asset::AssetPlugin {
            file_path: assets_dir().to_string_lossy().into_owned(),
            ..default()
        },
        TransformPlugin,
        WorldReloadPlugin,
        WorldRenderPlugin::new(render_tier()),
        WorldPhysicsPlugin,
    ));
    app.run_for(Duration::from_secs(20), |w| {
        w.resource::<ConfigRegistry>()
            .entries()
            .iter()
            .all(|e| e.applied + e.failures > 0)
    })
    .expect("configs never finished loading");

    let registry = app.world().resource::<ConfigRegistry>();
    for e in registry.entries() {
        assert_eq!(
            (e.origin, e.failures),
            (ConfigOrigin::File, 0),
            "{}: {:?}",
            e.name,
            e.last_error
        );
    }
    let registered: Vec<_> = registry.entries().iter().map(|e| e.path.clone()).collect();
    for file in std::fs::read_dir(assets_dir().join("config")).unwrap() {
        let name = file.unwrap().file_name().to_string_lossy().into_owned();
        if name.ends_with(".ron") {
            let path = format!("config/{name}");
            assert!(
                registered.contains(&path),
                "{path} is shipped but no plugin loads it"
            );
        }
    }
}

// ------------------------------------------------------------ game camera --

fn player_cameras(app: &mut HeadlessApp) -> Vec<Entity> {
    app.world_mut()
        .query_filtered::<Entity, With<PlayerCamera>>()
        .iter(app.world())
        .collect()
}

#[test]
fn the_player_camera_is_rebuilt_by_a_code_reload() {
    for_each_seed(|seed| {
        let mut app = HeadlessApp::new().randomized(seed);
        app.add_plugins((WorldReloadPlugin, WorldGamePlugin));
        app.step(2);
        let before = player_cameras(&mut app);
        assert_eq!(before.len(), 1, "seed {seed:#x}");

        app.world_mut().write_message(CodeReloaded);
        app.step(1);
        let after = player_cameras(&mut app);
        assert_eq!(after.len(), 1, "seed {seed:#x}: cameras accumulated");
        assert_ne!(after, before, "seed {seed:#x}: the camera was not rebuilt");
        let sky = app.world().get::<SkyComposite>(after[0]);
        assert!(sky.is_some(), "the rebuilt camera lost its sky composite");
    });
}

// ---------------------------------------------------------------- shaders --

const SIZE: u32 = 32;
const DEADLINE: Duration = Duration::from_secs(45);
const COMMON_SKY: &str = "shaders/common/sky.wesl";

/// A watched temporary asset folder holding copies of the real sky shaders.
fn shader_folder(label: &str) -> DiskAssets {
    let disk = DiskAssets::new(label);
    for path in [SKY_SHADER_PATH, COMMON_SKY] {
        disk.write(path, &shipped(path));
    }
    disk
}

fn sky_app(assets: bevy::asset::AssetPlugin) -> Option<OffscreenApp> {
    let adapter = TestAdapter::from_env();
    let built = OffscreenApp::with_asset_plugin(adapter, SIZE, SIZE, assets, |app| {
        app.add_plugins(FullscreenMaterialPlugin::<SkyComposite>::default());
    });
    match built {
        Ok(mut app) => {
            let camera = app.spawn_camera(Color::BLACK);
            app.world_mut()
                .entity_mut(camera)
                .insert(SkyComposite::default());
            Some(app)
        }
        Err(RenderError::NoAdapter(kind)) => {
            skip_or_fail(&format!("no {kind:?} adapter for offscreen rendering"));
            None
        }
        Err(other) => panic!("could not start the offscreen renderer: {other:?}"),
    }
}

/// Captures frames until `want` holds for one, or fails after the deadline.
fn capture_until(
    app: &mut OffscreenApp,
    what: &str,
    want: impl Fn(&Rgba8Image) -> bool,
) -> Rgba8Image {
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < DEADLINE {
        let frame = app.capture().expect("capture failed");
        if want(&frame) {
            return frame;
        }
        last = Some(frame.pixel(SIZE / 2, 0));
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{what}: never seen within {DEADLINE:?}; last top pixel {last:?}");
}

fn close(a: [u8; 4], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

fn top(frame: &Rgba8Image) -> [u8; 4] {
    frame.pixel(SIZE / 2, 0)
}

fn bottom(frame: &Rgba8Image) -> [u8; 4] {
    frame.pixel(SIZE / 2, SIZE - 1)
}

fn is_sky(frame: &Rgba8Image) -> bool {
    let (t, b) = (top(frame), bottom(frame));
    // Bluer at the top than at the horizon, and opaque.
    t[3] == 255 && t[2] > t[0] + 60 && b[0] > t[0] + 40
}

#[test]
fn gpu_the_sky_composite_matches_its_golden_image() {
    let Some(mut app) = sky_app(bevy::asset::AssetPlugin {
        file_path: assets_dir().to_string_lossy().into_owned(),
        ..default()
    }) else {
        return;
    };
    let frame = capture_until(&mut app, "the sky", is_sky);
    assert_image_golden(
        &world_tests::golden("sky_composite.png"),
        &frame,
        match TestAdapter::from_env() {
            TestAdapter::Software => Tolerance::per_channel(1),
            TestAdapter::Gpu => Tolerance::per_channel(3),
        },
    );
}

#[test]
fn gpu_editing_an_imported_shader_module_reloads_the_pass() {
    let disk = shader_folder("shader_import");
    let Some(mut app) = sky_app(disk.plugin()) else {
        return;
    };
    capture_until(&mut app, "the original sky", is_sky);

    // Only the imported module changes: the zenith becomes pure green.
    let edited = shipped(COMMON_SKY).replace(
        "const ZENITH: vec3<f32> = vec3<f32>(0.22, 0.45, 0.85);",
        "const ZENITH: vec3<f32> = vec3<f32>(0.0, 1.0, 0.0);",
    );
    assert_ne!(
        edited,
        shipped(COMMON_SKY),
        "the zenith line moved; update this test"
    );
    disk.write(COMMON_SKY, &edited);
    let frame = capture_until(&mut app, "the green zenith", |f| {
        let t = top(f);
        t[1] > 240 && t[0] < 40 && t[2] < 40
    });
    // The horizon end of the gradient did not change.
    assert!(
        close(bottom(&frame), [229, 239, 249], 8),
        "{:?}",
        bottom(&frame)
    );
}

#[test]
fn gpu_a_broken_shader_edit_is_survived_and_a_fix_recovers() {
    let disk = shader_folder("shader_broken");
    let Some(mut app) = sky_app(disk.plugin()) else {
        return;
    };
    capture_until(&mut app, "the original sky", is_sky);

    // A syntax error: the pipeline fails to compile, the app must keep running.
    disk.write(SKY_SHADER_PATH, "this is not a shader {");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        app.capture()
            .expect("rendering stopped after a broken shader");
    }

    // A fixed shader is picked up again: solid magenta everywhere.
    let magenta = r#"
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
"#;
    disk.write(SKY_SHADER_PATH, magenta);
    let frame = capture_until(&mut app, "the fixed magenta shader", |f| {
        close(top(f), [255, 0, 255], 0) && close(bottom(f), [255, 0, 255], 0)
    });
    assert!(
        frame
            .data
            .chunks_exact(4)
            .all(|px| px == [255, 0, 255, 255])
    );
}
