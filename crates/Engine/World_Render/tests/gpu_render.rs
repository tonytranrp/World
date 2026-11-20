//! The whole plugin in an offscreen Bevy app: the built-in debug scene goes
//! through the feed, the brick cache streams it, the trace and the voxel
//! composite draw it, and the sky fills the rest. The frame is saved as a
//! PNG under `target/world_render/` as evidence.
//!
//! Smoke test with a light reference: the image must show the floor's grass
//! colour below the horizon and the sky above, and the CPU decoder is not
//! involved. Exactness is covered by `gpu_trace`.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use wgpu::{Backend, DeviceType, DownlevelFlags, Features, Limits};
use world_render::prelude::{
    AdapterReport, Budgets, Profile, RenderTier, SkyComposite, WorldRenderPlugin, classify,
};
use world_testkit::golden::Rgba8Image;
use world_testkit::policy::skip_or_fail;
use world_testkit_bevy::render::{OffscreenApp, RenderError, TestAdapter};

const W: u32 = 320;
const H: u32 = 180;
const DEADLINE: Duration = Duration::from_secs(90);

fn tier() -> RenderTier {
    let report = AdapterReport {
        name: "test adapter".into(),
        backend: Backend::Dx12,
        device_type: DeviceType::Cpu,
        features: Features::empty(),
        limits: Limits::default(),
        downlevel_flags: DownlevelFlags::all(),
    };
    let verdict = classify(&report).expect("tier B");
    RenderTier {
        tier: verdict.tier,
        capabilities: verdict.capabilities,
        // The floor profile keeps the pools small enough for the software adapter.
        profile: Profile::MacFloor,
        budgets: Budgets::for_profile(Profile::MacFloor, &report),
        backend: report.backend,
        adapter_name: report.name,
    }
}

/// Fraction of pixels close to `rgb` in a row band.
fn share_near(img: &Rgba8Image, rows: std::ops::Range<u32>, test: impl Fn([u8; 4]) -> bool) -> f32 {
    let mut n = 0;
    let mut hit = 0;
    for y in rows {
        for x in 0..img.width {
            n += 1;
            if test(img.pixel(x, y)) {
                hit += 1;
            }
        }
    }
    hit as f32 / n.max(1) as f32
}

/// Grass lit from above: green clearly above red and blue.
fn grassy(p: [u8; 4]) -> bool {
    p[1] > 60 && p[1] as i32 > p[0] as i32 + 20 && p[1] as i32 > p[2] as i32 + 20
}

/// The sky gradient: blue above red.
fn sky(p: [u8; 4]) -> bool {
    p[2] as i32 > p[0] as i32 + 15 && p[2] > 120
}

#[test]
fn gpu_the_debug_scene_renders_through_the_brick_cache() {
    // The offscreen app installs no log subscriber; show warnings and errors.
    let _ = bevy::log::tracing_subscriber::fmt()
        .with_env_filter(
            bevy::log::tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,world_render=info".into()),
        )
        .with_test_writer()
        .try_init();
    let adapter = TestAdapter::from_env();
    let built = OffscreenApp::new(adapter, W, H, |app| {
        app.add_plugins(WorldRenderPlugin::new(tier()));
    });
    let mut app = match built {
        Ok(app) => app,
        Err(RenderError::NoAdapter(kind)) => {
            skip_or_fail(&format!("no {kind:?} adapter for offscreen rendering"));
            return;
        }
        Err(other) => panic!("could not start the offscreen renderer: {other:?}"),
    };
    let camera = app.spawn_camera(Color::BLACK);
    app.world_mut()
        .entity_mut(camera)
        .insert(SkyComposite::default());

    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < DEADLINE {
        let frame = app.capture().expect("capture");
        let ground = share_near(&frame, H * 3 / 4..H, grassy);
        let top = share_near(&frame, 0..H / 6, sky);
        if ground > 0.3 && top > 0.8 {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../target/world_render")
                .join(format!("debug_scene_{adapter:?}.png").to_lowercase());
            frame.save(&path).expect("save evidence");
            eprintln!(
                "voxels rendered after {:.1} s: {:.0}% grass in the bottom quarter, saved {}",
                start.elapsed().as_secs_f32(),
                ground * 100.0,
                path.display()
            );
            return;
        }
        last = Some((ground, top));
    }
    panic!("the debug scene never appeared (grass share, sky share): {last:?}");
}
