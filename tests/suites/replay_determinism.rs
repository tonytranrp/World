//! Deterministic replay: the same scenario must hash identically at every step,
//! under every system order, and match the recorded replay.

use bevy::prelude::*;
use world_testkit::hash::StateHasher;
use world_testkit::replay::{Replay, assert_deterministic, assert_replay_golden};
use world_testkit_bevy::app::HeadlessApp;
use world_testkit_bevy::schedule::for_each_seed;

const SCENARIO: &str = "falling_bodies";
const STEPS: usize = 120;
const BODIES: u32 = 64;

#[derive(Component)]
struct Body {
    pos: Vec3,
    vel: Vec3,
}

/// Gravity with a damped bounce off the ground plane, at a fixed step.
fn simulate(mut bodies: Query<&mut Body>) {
    const DT: f32 = 1.0 / 60.0;
    for mut body in &mut bodies {
        body.vel.y -= 9.81 * DT;
        let velocity = body.vel;
        body.pos += velocity * DT;
        if body.pos.y < 0.0 {
            body.pos.y = 0.0;
            body.vel.y *= -0.5;
        }
    }
}

fn spawn_bodies(mut commands: Commands) {
    // A fixed generator, so every run starts from the same state.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    for _ in 0..BODIES {
        commands.spawn(Body {
            pos: Vec3::new(next() * 10.0, 5.0 + next() * 20.0, next() * 10.0),
            vel: Vec3::new(next() - 0.5, 0.0, next() - 0.5),
        });
    }
}

fn state_hash(world: &mut World) -> u64 {
    let mut hasher = StateHasher::new();
    for body in world.query::<&Body>().iter(world) {
        for c in body.pos.to_array().into_iter().chain(body.vel.to_array()) {
            hasher.write_f32(c);
        }
    }
    hasher.finish()
}

fn run(seed: Option<u64>) -> impl FnMut(usize) -> u64 {
    let mut app = HeadlessApp::new();
    if let Some(seed) = seed {
        app = app.randomized(seed);
    }
    app.app_mut()
        .add_systems(Startup, spawn_bodies)
        .add_systems(Update, simulate);
    move |_| {
        app.step(1);
        state_hash(app.world_mut())
    }
}

#[test]
fn the_scenario_is_deterministic() {
    assert_deterministic(SCENARIO, STEPS, || run(None));
}

#[test]
fn the_result_does_not_depend_on_system_order() {
    let reference = Replay::record(SCENARIO, STEPS, run(None));
    for_each_seed(|seed| {
        let shuffled = Replay::record(SCENARIO, STEPS, run(Some(seed)));
        assert_eq!(
            reference.first_difference(&shuffled),
            None,
            "seed {seed:#x}"
        );
    });
}

#[test]
fn the_scenario_matches_its_recorded_replay() {
    let actual = Replay::record(SCENARIO, STEPS, run(None));
    assert_replay_golden(&world_tests::replay("falling_bodies.replay"), &actual);
}

#[test]
fn the_state_actually_changes_over_time() {
    let replay = Replay::record(SCENARIO, STEPS, run(None));
    assert!(
        replay.hashes.windows(2).all(|w| w[0] != w[1]),
        "a step left the state unchanged"
    );
}
