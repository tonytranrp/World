//! Headless render compared with a golden image.
//!
//! Runs on the software adapter by default so the image is identical on every
//! machine. `WORLD_TEST_ADAPTER=gpu` runs the same test on the real GPU with a
//! looser tolerance. The `gpu_` prefix puts these tests in the serialized
//! nextest group.

use bevy::prelude::*;
use world_testkit::golden::{Tolerance, assert_image_golden};
use world_testkit::policy::skip_or_fail;
use world_testkit_bevy::render::{OffscreenApp, RenderError, TestAdapter};

fn tolerance(adapter: TestAdapter) -> Tolerance {
    match adapter {
        TestAdapter::Software => Tolerance::per_channel(1),
        TestAdapter::Gpu => Tolerance::per_channel(3),
    }
}

fn offscreen(width: u32, height: u32) -> Option<(OffscreenApp, TestAdapter)> {
    let adapter = TestAdapter::from_env();
    match OffscreenApp::new(adapter, width, height, |_| {}) {
        Ok(app) => Some((app, adapter)),
        Err(RenderError::NoAdapter(kind)) => {
            skip_or_fail(&format!(
                "no {kind:?} adapter available for offscreen rendering"
            ));
            None
        }
        Err(other) => panic!("could not start the offscreen renderer: {other:?}"),
    }
}

#[test]
fn gpu_clear_colour_matches_the_golden_image() {
    let Some((mut app, adapter)) = offscreen(64, 64) else {
        return;
    };
    app.spawn_camera(Color::srgb(0.2, 0.4, 0.6));
    let frame = app.capture().expect("capture failed");
    assert_image_golden(
        &world_tests::golden("clear_colour.png"),
        &frame,
        tolerance(adapter),
    );
}

#[test]
fn gpu_every_pixel_of_a_clear_is_the_same_colour() {
    let Some((mut app, _)) = offscreen(32, 32) else {
        return;
    };
    app.spawn_camera(Color::srgb(1.0, 0.0, 0.0));
    let frame = app.capture().expect("capture failed");
    let first = frame.pixel(0, 0);
    assert_eq!(first[3], 255);
    assert!(
        frame.data.chunks_exact(4).all(|px| px == first),
        "the clear was not uniform"
    );
    assert!(
        first[0] >= 254 && first[1] <= 1 && first[2] <= 1,
        "expected red, got {first:?}"
    );
}

#[test]
fn gpu_different_clear_colours_give_different_images() {
    let Some((mut a, _)) = offscreen(16, 16) else {
        return;
    };
    a.spawn_camera(Color::srgb(0.0, 1.0, 0.0));
    let green = a.capture().expect("capture failed");
    let Some((mut b, _)) = offscreen(16, 16) else {
        return;
    };
    b.spawn_camera(Color::srgb(0.0, 0.0, 1.0));
    let blue = b.capture().expect("capture failed");
    assert_ne!(green.pixel(8, 8), blue.pixel(8, 8));
}
