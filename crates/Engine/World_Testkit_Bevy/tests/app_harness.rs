use bevy::prelude::*;
use world_testkit_bevy::app::{HeadlessApp, StepsExhausted};

#[derive(Resource, Default)]
struct Frames(u32);

fn count_frame(mut frames: ResMut<Frames>) {
    frames.0 += 1;
}

fn app() -> HeadlessApp {
    let mut app = HeadlessApp::new();
    app.app_mut()
        .init_resource::<Frames>()
        .add_systems(Update, count_frame);
    app
}

#[test]
fn step_runs_exactly_that_many_frames() {
    let mut app = app();
    app.step(5);
    assert_eq!(app.world().resource::<Frames>().0, 5);
    app.step(2);
    assert_eq!(app.world().resource::<Frames>().0, 7);
}

#[test]
fn startup_runs_once_before_the_first_update() {
    #[derive(Resource, Default)]
    struct Log(Vec<&'static str>);

    let mut app = HeadlessApp::new();
    app.app_mut()
        .init_resource::<Log>()
        .add_systems(Startup, |mut log: ResMut<Log>| log.0.push("startup"))
        .add_systems(Update, |mut log: ResMut<Log>| log.0.push("update"));
    app.step(2);
    assert_eq!(
        app.world().resource::<Log>().0,
        ["startup", "update", "update"]
    );
}

#[test]
fn run_until_reports_how_many_frames_it_took() {
    let mut app = app();
    let taken = app.run_until(100, |w| w.resource::<Frames>().0 >= 7);
    assert_eq!(taken, Ok(7));
}

#[test]
fn run_until_gives_up_after_max_steps() {
    let mut app = app();
    assert_eq!(
        app.run_until(10, |w| w.resource::<Frames>().0 >= 1_000),
        Err(StepsExhausted { steps: 10 })
    );
}

#[test]
fn world_mut_changes_are_seen_by_systems() {
    let mut app = app();
    app.world_mut().resource_mut::<Frames>().0 = 100;
    app.step(1);
    assert_eq!(app.world().resource::<Frames>().0, 101);
}
