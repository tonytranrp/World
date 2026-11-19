//! Proves that schedule randomization exposes a hidden ordering assumption,
//! and leaves explicit ordering alone.

use std::collections::BTreeSet;

use bevy::prelude::*;
use world_testkit_bevy::app::HeadlessApp;
use world_testkit_bevy::schedule::for_each_seed;

#[derive(Resource, Default)]
struct Log(Vec<&'static str>);

fn a(mut log: ResMut<Log>) {
    log.0.push("a");
}

fn b(mut log: ResMut<Log>) {
    log.0.push("b");
}

fn run_once(seed: u64, ordered: bool) -> Vec<&'static str> {
    let mut app = HeadlessApp::new().randomized(seed);
    app.app_mut().init_resource::<Log>();
    if ordered {
        app.app_mut().add_systems(Update, (a, b).chain());
    } else {
        app.app_mut().add_systems(Update, (a, b));
    }
    app.step(1);
    app.world().resource::<Log>().0.clone()
}

#[test]
fn unordered_systems_run_in_both_orders_across_seeds() {
    let orders: BTreeSet<_> = (0..32u64).map(|seed| run_once(seed, false)).collect();
    assert!(
        orders.contains(&vec!["a", "b"]) && orders.contains(&vec!["b", "a"]),
        "expected both orders, saw {orders:?}"
    );
}

#[test]
fn chained_systems_keep_their_order_under_every_seed() {
    for_each_seed(|seed| assert_eq!(run_once(seed, true), ["a", "b"], "seed {seed:#x}"));
}

#[test]
fn the_same_seed_gives_the_same_order() {
    for seed in 0..8u64 {
        assert_eq!(run_once(seed, false), run_once(seed, false));
    }
}

/// Regression: swapping the executor once dropped the final apply of deferred
/// commands, so entities spawned in `Startup` never existed.
#[test]
fn commands_queued_in_startup_are_applied_under_every_seed() {
    #[derive(Component)]
    struct Marker;

    #[derive(Resource, Default)]
    struct Seen(usize);

    for_each_seed(|seed| {
        let mut app = HeadlessApp::new().randomized(seed);
        app.app_mut()
            .init_resource::<Seen>()
            .add_systems(Startup, |mut commands: Commands| {
                commands.spawn(Marker);
                commands.spawn(Marker);
            })
            .add_systems(Update, |markers: Query<&Marker>, mut seen: ResMut<Seen>| {
                seen.0 = markers.iter().count();
            });
        app.step(2);
        assert_eq!(app.world().resource::<Seen>().0, 2, "seed {seed:#x}");
    });
}
