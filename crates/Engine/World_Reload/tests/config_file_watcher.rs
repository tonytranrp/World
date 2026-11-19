//! Config hot reload end to end on a real folder with Bevy's real file
//! watcher, including its 300 ms debounce. Slower than the in-memory suite, so
//! it covers what only the real path can show: editor-style saves, atomic
//! renames, the panel's save path, and deleting and recreating a file.

use std::time::Duration;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use world_reload::{
    AppConfigExt, Config, ConfigAsset, ConfigOrigin, ConfigRegistry, WorldReloadPlugin,
};
use world_testkit_bevy::app::HeadlessApp;
use world_testkit_bevy::assets::DiskAssets;

/// Generous: CI machines can be slow to deliver file events.
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gravity {
    metres_per_second_squared: f32,
}

impl ConfigAsset for Gravity {
    const NAME: &'static str = "gravity";
    fn validate(&self) -> Result<(), Vec<String>> {
        if (0.0..=100.0).contains(&self.metres_per_second_squared) {
            Ok(())
        } else {
            Err(vec!["out of range".into()])
        }
    }
}

const PATH: &str = "config/gravity.ron";

fn text(v: f32) -> String {
    format!("// comment kept by the panel\n(metres_per_second_squared: {v:?})\n")
}

fn app(disk: &DiskAssets) -> HeadlessApp {
    let mut app = HeadlessApp::new();
    app.add_plugins(disk.plugin())
        .add_plugins(WorldReloadPlugin);
    app.app_mut().add_config::<Gravity>(PATH);
    app
}

fn live(world: &World) -> f32 {
    world
        .resource::<Config<Gravity>>()
        .metres_per_second_squared
}

fn wait_for_value(app: &mut HeadlessApp, value: f32) {
    app.run_for(TIMEOUT, |w| live(w) == value)
        .unwrap_or_else(|t| {
            panic!(
                "value {value} never became live: {t:?}; now {}",
                live(app_world(app))
            )
        });
}

fn app_world(app: &HeadlessApp) -> &World {
    app.world()
}

fn status(app: &HeadlessApp) -> world_reload::ConfigEntry {
    app.world()
        .resource::<ConfigRegistry>()
        .get("gravity")
        .cloned()
        .unwrap()
}

#[test]
fn an_edit_on_disk_reaches_the_running_app() {
    let disk = DiskAssets::new("watch_edit");
    disk.write(PATH, &text(9.81));
    let mut app = app(&disk);
    wait_for_value(&mut app, 9.81);

    disk.write(PATH, &text(3.7));
    wait_for_value(&mut app, 3.7);
    assert_eq!(status(&app).origin, ConfigOrigin::File);
}

#[test]
fn a_bad_edit_on_disk_is_rejected_then_a_fix_recovers() {
    let disk = DiskAssets::new("watch_bad");
    disk.write(PATH, &text(9.81));
    let mut app = app(&disk);
    wait_for_value(&mut app, 9.81);

    disk.write(PATH, "(metres_per_second_squared: 500.0)");
    app.run_for(TIMEOUT, |w| {
        w.resource::<ConfigRegistry>()
            .get("gravity")
            .unwrap()
            .failures
            >= 1
    })
    .expect("the bad edit was never reported");
    assert_eq!(live(app.world()), 9.81);

    disk.write(PATH, &text(1.62));
    wait_for_value(&mut app, 1.62);
    assert_eq!(status(&app).last_error, None);
}

#[test]
fn rapid_saves_settle_on_the_final_contents() {
    let disk = DiskAssets::new("watch_burst");
    disk.write(PATH, &text(1.0));
    let mut app = app(&disk);
    wait_for_value(&mut app, 1.0);

    // Faster than the debounce window, as an editor with autosave would.
    for i in 2..=12 {
        disk.write(PATH, &text(i as f32));
        std::thread::sleep(Duration::from_millis(15));
    }
    wait_for_value(&mut app, 12.0);
    // Let any late events land; the value must not move.
    let _ = app.run_for(Duration::from_millis(800), |_| false);
    assert_eq!(live(app.world()), 12.0);
}

#[test]
fn saving_from_the_panel_writes_the_file_and_keeps_comments() {
    let disk = DiskAssets::new("watch_save");
    disk.write(PATH, &text(9.81));
    let mut app = app(&disk);
    wait_for_value(&mut app, 9.81);
    assert_eq!(
        app.world()
            .resource::<ConfigRegistry>()
            .file_path("gravity"),
        Some(disk.root().join(PATH))
    );

    let edited = text(5.0);
    let written = ConfigRegistry::save_text(app.world_mut(), "gravity", &edited).unwrap();
    assert_eq!(written, disk.root().join(PATH));
    // Applied at once, and the file holds exactly the editor text, comment included.
    assert_eq!(live(app.world()), 5.0);
    assert_eq!(disk.read(PATH), edited);
    assert!(!disk.root().join("config/gravity.ron.tmp").exists());

    // The watcher then sees the save and reports the value as coming from the file.
    app.run_for(TIMEOUT, |w| {
        w.resource::<ConfigRegistry>()
            .get("gravity")
            .unwrap()
            .origin
            == ConfigOrigin::File
    })
    .expect("the saved file was never reloaded");
    assert_eq!(live(app.world()), 5.0);

    // Invalid text is never written.
    assert!(ConfigRegistry::save_text(app.world_mut(), "gravity", &text(-3.0)).is_err());
    assert_eq!(disk.read(PATH), edited);
}

#[test]
fn deleting_and_recreating_the_file_recovers() {
    let disk = DiskAssets::new("watch_delete");
    disk.write(PATH, &text(2.0));
    let mut app = app(&disk);
    wait_for_value(&mut app, 2.0);

    disk.remove(PATH);
    let _ = app.run_for(Duration::from_millis(800), |_| false);
    assert_eq!(live(app.world()), 2.0, "deleting the file lost the value");

    disk.write(PATH, &text(4.0));
    wait_for_value(&mut app, 4.0);
}
