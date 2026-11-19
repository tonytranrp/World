//! Engine plugins driven through a headless App, under shuffled system order.

use bevy::prelude::*;
use wgpu::{Backend, DeviceType, DownlevelFlags, Features, Limits};
use world_render::prelude::{
    AdapterReport, Budgets, Profile, RenderTier, Tier, WorldRenderPlugin, classify,
};
use world_testkit_bevy::app::HeadlessApp;
use world_testkit_bevy::schedule::for_each_seed;

fn adapter(features: Features) -> AdapterReport {
    AdapterReport {
        name: "test adapter".into(),
        backend: Backend::Vulkan,
        device_type: DeviceType::DiscreteGpu,
        features,
        limits: Limits::default(),
        downlevel_flags: DownlevelFlags::all(),
    }
}

fn render_tier(report: &AdapterReport, profile: Profile) -> RenderTier {
    let verdict = classify(report).unwrap().with_profile(profile);
    RenderTier {
        tier: verdict.tier,
        capabilities: verdict.capabilities,
        profile,
        budgets: Budgets::for_profile(profile, report),
        backend: report.backend,
        adapter_name: report.name.clone(),
    }
}

const RT: Features = Features::EXPERIMENTAL_RAY_QUERY.union(Features::EXPERIMENTAL_MESH_SHADER);

#[test]
fn the_render_plugin_publishes_the_tier_it_was_given() {
    for_each_seed(|seed| {
        let tier = render_tier(&adapter(RT), Profile::Full);
        let mut app = HeadlessApp::new().randomized(seed);
        app.add_plugins(WorldRenderPlugin::new(tier));
        app.step(2);
        let published = app.world().resource::<RenderTier>();
        assert_eq!(published.tier, Tier::A, "seed {seed:#x}");
        assert_eq!(published.adapter_name, "test adapter");
    });
}

#[test]
fn mac_floor_publishes_tier_b_with_the_small_budgets_on_a_tier_a_adapter() {
    let tier = render_tier(&adapter(RT), Profile::MacFloor);
    let mut app = HeadlessApp::new();
    app.add_plugins(WorldRenderPlugin::new(tier));
    app.step(1);
    let published = app.world().resource::<RenderTier>();
    assert_eq!(published.tier, Tier::B);
    assert!(!published.capabilities.ray_query && !published.capabilities.mesh_shader);
    assert_eq!(published.budgets.internal_resolution, UVec2::new(1280, 720));
    assert_eq!(published.budgets.worker_threads, Some(4));
}

#[test]
fn the_plugin_runs_without_a_gpu_present() {
    // No RenderPlugin, no device: the startup system must cope and not panic.
    let mut app = HeadlessApp::new();
    app.add_plugins(WorldRenderPlugin::new(render_tier(
        &adapter(Features::empty()),
        Profile::Full,
    )));
    app.step(3);
    assert_eq!(app.world().resource::<RenderTier>().tier, Tier::B);
}
