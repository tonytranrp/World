//! Reloadable setup: runs at startup, and after each `CodeReloaded` the old
//! entities go and fresh ones are built, under every shuffled system order.

use bevy::prelude::*;
use world_reload::{AppReloadExt, CodeReloadStats, CodeReloaded, ReloadScoped, WorldReloadPlugin};
use world_testkit_bevy::app::HeadlessApp;
use world_testkit_bevy::schedule::for_each_seed;

#[derive(Component)]
struct Camera;

#[derive(Component)]
struct Lamp;

#[derive(Component)]
struct Persistent;

#[derive(Resource, Default)]
struct SetupRuns(u32);

fn spawn_rig(mut commands: Commands, mut runs: ResMut<SetupRuns>) {
    runs.0 += 1;
    commands
        .spawn((Camera, ReloadScoped))
        .with_children(|parent| {
            parent.spawn(Lamp);
        });
}

fn app(seed: u64) -> HeadlessApp {
    let mut app = HeadlessApp::new().randomized(seed);
    app.add_plugins(WorldReloadPlugin);
    app.app_mut()
        .init_resource::<SetupRuns>()
        .add_reloadable_systems(|| spawn_rig);
    app
}

fn count<C: Component>(app: &mut HeadlessApp) -> usize {
    app.world_mut()
        .query_filtered::<(), With<C>>()
        .iter(app.world())
        .count()
}

fn camera(app: &mut HeadlessApp) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<Camera>>()
        .single(app.world())
        .unwrap()
}

fn reload(app: &mut HeadlessApp) {
    app.world_mut().write_message(CodeReloaded);
    app.step(1);
}

#[test]
fn setup_runs_once_at_startup_and_not_again_on_its_own() {
    for_each_seed(|seed| {
        let mut app = app(seed);
        app.step(30);
        assert_eq!(app.world().resource::<SetupRuns>().0, 1, "seed {seed:#x}");
        assert_eq!(count::<Camera>(&mut app), 1);
        assert_eq!(app.world().resource::<CodeReloadStats>().reloads, 0);
    });
}

#[test]
fn a_reload_replaces_scoped_entities_and_their_children() {
    for_each_seed(|seed| {
        let mut app = app(seed);
        app.step(2);
        let before = camera(&mut app);
        reload(&mut app);
        let after = camera(&mut app);
        assert_ne!(before, after, "seed {seed:#x}: the camera was not rebuilt");
        assert!(
            app.world().get_entity(before).is_err(),
            "seed {seed:#x}: old camera survived"
        );
        assert_eq!(count::<Camera>(&mut app), 1, "seed {seed:#x}");
        assert_eq!(
            count::<Lamp>(&mut app),
            1,
            "seed {seed:#x}: children leaked"
        );
        assert_eq!(app.world().resource::<SetupRuns>().0, 2);
        assert_eq!(app.world().resource::<CodeReloadStats>().reloads, 1);
    });
}

#[test]
fn entities_without_the_marker_survive_a_reload() {
    for_each_seed(|seed| {
        let mut app = app(seed);
        app.step(1);
        let kept = app.world_mut().spawn(Persistent).id();
        reload(&mut app);
        assert!(app.world().get_entity(kept).is_ok(), "seed {seed:#x}");
    });
}

#[test]
fn many_reloads_never_accumulate_entities() {
    for_each_seed(|seed| {
        let mut app = app(seed);
        app.step(1);
        for i in 1..=20 {
            reload(&mut app);
            app.step(i % 3);
        }
        assert_eq!(count::<Camera>(&mut app), 1, "seed {seed:#x}");
        assert_eq!(count::<Lamp>(&mut app), 1, "seed {seed:#x}");
        assert_eq!(app.world().resource::<SetupRuns>().0, 21);
        assert_eq!(app.world().resource::<CodeReloadStats>().reloads, 20);
    });
}

#[test]
fn reloadable_setup_without_the_plugin_still_runs_only_on_reloads() {
    for_each_seed(|seed| {
        let mut app = HeadlessApp::new().randomized(seed);
        app.app_mut()
            .init_resource::<SetupRuns>()
            .add_reloadable_systems(|| spawn_rig);
        app.step(10);
        assert_eq!(app.world().resource::<SetupRuns>().0, 1, "seed {seed:#x}");
        reload(&mut app);
        assert_eq!(app.world().resource::<SetupRuns>().0, 2, "seed {seed:#x}");
        assert_eq!(count::<Camera>(&mut app), 1, "seed {seed:#x}");
    });
}

#[test]
fn several_reload_messages_in_one_frame_rebuild_once() {
    let mut app = app(0);
    app.step(1);
    for _ in 0..5 {
        app.world_mut().write_message(CodeReloaded);
    }
    app.step(1);
    assert_eq!(app.world().resource::<SetupRuns>().0, 2);
    assert_eq!(count::<Camera>(&mut app), 1);
    app.step(5);
    assert_eq!(app.world().resource::<SetupRuns>().0, 2);
}

/// With the `hotpatch` feature, Bevy's own `HotPatched` message drives the same
/// path. Run with `cargo test -p world_reload --features hotpatch`.
#[cfg(feature = "hotpatch")]
#[test]
fn a_bevy_hot_patch_triggers_the_reload() {
    // No HotPatchPlugin here: the bridge must register the message itself.
    let mut app = app(0);
    app.step(1);
    let before = camera(&mut app);
    app.world_mut().write_message(bevy::ecs::HotPatched);
    // Forwarded in `First` of the next frame, rebuilt in its `PreUpdate`.
    app.step(2);
    assert_ne!(camera(&mut app), before);
    assert_eq!(app.world().resource::<CodeReloadStats>().reloads, 1);
}
