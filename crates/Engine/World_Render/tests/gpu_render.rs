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

/// Viewport of the stretch test, and its internal resolution: the profile
/// caps the height at 120, so the trace is 213x120 (the viewport's aspect,
/// rounded), a ratio that is not a whole number on either axis.
const STRETCH_VIEWPORT: (u32, u32) = (320, 180);
const STRETCH_TRACE: (u32, u32) = (213, 120);

/// Linear to sRGB, as an 8-bit unorm sRGB target stores it.
fn linear_to_srgb8(c: f32) -> u8 {
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Every colour the voxel composite can write: each material's colour
/// times each face's light (the shading's `face_light`), plus the magenta
/// of an unknown material.
fn voxel_palette(app: &mut OffscreenApp) -> Vec<[u8; 3]> {
    const FACE_LIGHT: [f32; 7] = [0.72, 0.80, 0.45, 1.00, 0.62, 0.88, 1.0];
    let table = app
        .world_mut()
        .resource::<world_reload::Config<world_core::MaterialTable>>();
    let mut colors: Vec<[f32; 3]> = table
        .iter()
        .map(|m| {
            [
                srgb_to_linear(m.color.0),
                srgb_to_linear(m.color.1),
                srgb_to_linear(m.color.2),
            ]
        })
        .collect();
    colors.push([1.0, 0.0, 1.0]);
    let mut palette = Vec::new();
    for c in colors {
        for light in FACE_LIGHT {
            palette.push(c.map(|v| linear_to_srgb8(v * light)));
        }
    }
    palette
}

/// Below the viewport's size (the profile's internal resolution) the trace
/// is stretched over the viewport, each screen pixel showing the traced
/// pixel under it. Without the sky, voxel pixels keep the composite's alpha
/// 0 and misses the opaque black clear, so the check sees exactly what the
/// voxel composite wrote: every screen pixel over one traced pixel has the
/// same colour, and every covered pixel is a material colour times a face
/// light. The frame is saved under `target/world_render/` as evidence.
#[test]
fn gpu_the_composite_stretches_a_smaller_trace_over_the_viewport() {
    let _ = bevy::log::tracing_subscriber::fmt()
        .with_env_filter(
            bevy::log::tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,world_render=info".into()),
        )
        .with_test_writer()
        .try_init();
    let (w, h) = STRETCH_VIEWPORT;
    let adapter = TestAdapter::from_env();
    let mut tier = tier();
    tier.budgets.internal_resolution = UVec2::new(10_000, STRETCH_TRACE.1);
    let built = OffscreenApp::new(adapter, w, h, |app| {
        app.add_plugins(WorldRenderPlugin::new(tier));
    });
    let mut app = match built {
        Ok(app) => app,
        Err(RenderError::NoAdapter(kind)) => {
            skip_or_fail(&format!("no {kind:?} adapter for offscreen rendering"));
            return;
        }
        Err(other) => panic!("could not start the offscreen renderer: {other:?}"),
    };
    app.spawn_camera(Color::BLACK);

    let start = Instant::now();
    let frame = loop {
        let frame = app.capture().expect("capture");
        let ground = share_near(&frame, h * 3 / 4..h, |p| p[3] == 0 && grassy(p));
        if ground > 0.3 {
            break frame;
        }
        assert!(
            start.elapsed() < DEADLINE,
            "the debug scene never appeared (grass share {ground})"
        );
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/world_render")
        .join(format!("stretched_{adapter:?}.png").to_lowercase());
    frame.save(&path).expect("save evidence");

    let (tw, th) = STRETCH_TRACE;
    let mut under: Vec<Option<[u8; 4]>> = vec![None; (tw * th) as usize];
    let palette = voxel_palette(&mut app);
    let (mut covered, mut misses) = (0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            let p = frame.pixel(x, y);
            let (sx, sy) = ((x * tw / w).min(tw - 1), (y * th / h).min(th - 1));
            let first = under[(sy * tw + sx) as usize].get_or_insert(p);
            assert_eq!(
                *first, p,
                "screen pixel ({x}, {y}) differs from another over traced pixel ({sx}, {sy})"
            );
            match p[3] {
                0 => {
                    covered += 1;
                    assert!(
                        palette
                            .iter()
                            .any(|c| (0..3).all(|i| c[i].abs_diff(p[i]) <= 1)),
                        "screen pixel ({x}, {y}) {p:?} is no material colour times a face light"
                    );
                }
                255 => {
                    misses += 1;
                    assert_eq!(&p[..3], &[0, 0, 0], "a miss keeps the clear colour");
                }
                a => panic!("screen pixel ({x}, {y}) has alpha {a}"),
            }
        }
    }
    assert!(
        covered > 0 && misses > 0,
        "{covered} covered, {misses} misses"
    );
    eprintln!(
        "stretched {tw}x{th} over {w}x{h}: {covered} covered, {misses} misses, saved {}",
        path.display()
    );
}
