//! Config hot reload through an in-memory asset source with a hand-driven
//! watcher: deterministic, no file system, no debounce.

use std::time::Duration;

use bevy::prelude::*;
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use world_reload::{
    AppConfigExt, Config, ConfigAsset, ConfigError, ConfigOrigin, ConfigRegistry, WorldReloadPlugin,
};
use world_testkit_bevy::app::HeadlessApp;
use world_testkit_bevy::assets::MemoryAssets;
use world_testkit_bevy::schedule::for_each_seed;

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Speed {
    metres_per_second: f32,
}

impl ConfigAsset for Speed {
    const NAME: &'static str = "speed";
    fn validate(&self) -> Result<(), Vec<String>> {
        if self.metres_per_second.is_finite() && self.metres_per_second >= 0.0 {
            Ok(())
        } else {
            Err(vec![format!(
                "metres_per_second = {} must be finite and not negative",
                self.metres_per_second
            )])
        }
    }
}

/// A second config type, also a `.ron`, to prove loaders never mix files up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    text: String,
}

impl ConfigAsset for Label {
    const NAME: &'static str = "label";
}

const SPEED: &str = "config/speed.ron";
const LABEL: &str = "config/label.ron";

fn speed(v: f32) -> String {
    format!("(metres_per_second: {v:?})")
}

/// An app with the speed config, its file already present, not yet stepped.
fn app_with(seed: Option<u64>, initial: Option<&str>) -> (HeadlessApp, MemoryAssets) {
    let mut app = match seed {
        Some(seed) => HeadlessApp::new().randomized(seed),
        None => HeadlessApp::new(),
    };
    let assets = MemoryAssets::install(app.app_mut());
    if let Some(text) = initial {
        assets.put(SPEED, text);
    }
    app.add_plugins(WorldReloadPlugin);
    app.app_mut().add_config::<Speed>(SPEED);
    (app, assets)
}

fn entry(app: &HeadlessApp, name: &str) -> world_reload::ConfigEntry {
    app.world()
        .resource::<ConfigRegistry>()
        .get(name)
        .cloned()
        .unwrap()
}

fn live(app: &HeadlessApp) -> f32 {
    app.world().resource::<Config<Speed>>().metres_per_second
}

/// Steps until the config has been applied or rejected `count` times in total.
fn settle(app: &mut HeadlessApp, name: &str, count: u32) {
    app.run_for(TIMEOUT, |w| {
        let e = w.resource::<ConfigRegistry>().get(name).unwrap().clone();
        e.applied + e.failures >= count
    })
    .unwrap_or_else(|t| panic!("{name} did not settle at {count}: {t:?}"));
}

#[test]
fn defaults_are_live_before_the_file_loads() {
    let (mut app, _assets) = app_with(None, Some(&speed(3.0)));
    app.step(0);
    assert_eq!(live(&app), 0.0);
    assert_eq!(entry(&app, "speed").origin, ConfigOrigin::Defaults);
}

#[test]
fn the_file_is_applied_at_startup() {
    for_each_seed(|seed| {
        let (mut app, _assets) = app_with(Some(seed), Some(&speed(3.0)));
        settle(&mut app, "speed", 1);
        assert_eq!(live(&app), 3.0, "seed {seed:#x}");
        let e = entry(&app, "speed");
        assert_eq!(
            (e.origin, e.applied, e.failures),
            (ConfigOrigin::File, 1, 0)
        );
    });
}

#[test]
fn a_saved_change_is_applied_without_restarting() {
    let (mut app, assets) = app_with(None, Some(&speed(3.0)));
    settle(&mut app, "speed", 1);
    assets.write(SPEED, &speed(9.5));
    settle(&mut app, "speed", 2);
    assert_eq!(live(&app), 9.5);
    assert_eq!(entry(&app, "speed").applied, 2);
}

#[test]
fn a_bad_save_is_rejected_and_the_last_good_value_stays() {
    let (mut app, assets) = app_with(None, Some(&speed(3.0)));
    settle(&mut app, "speed", 1);

    for (i, bad) in [
        "(metres_per_second: )",          // syntax
        "(metres_per_second: -2.0)",      // validation
        "(metres_per_second: 1.0, x: 2)", // unknown field
        "(speed: 1.0)",                   // wrong field
        "",                               // empty file
    ]
    .into_iter()
    .enumerate()
    {
        assets.write(SPEED, bad);
        settle(&mut app, "speed", 2 + i as u32);
        assert_eq!(live(&app), 3.0, "{bad:?} changed the live value");
        let e = entry(&app, "speed");
        assert_eq!(e.failures, 1 + i as u32);
        assert!(e.last_error.is_some(), "{bad:?} left no error");
    }

    // Fixing the file recovers and clears the error.
    assets.write(SPEED, &speed(4.0));
    settle(&mut app, "speed", 7);
    assert_eq!(live(&app), 4.0);
    let e = entry(&app, "speed");
    assert_eq!(e.last_error, None);
    assert_eq!(e.origin, ConfigOrigin::File);
}

#[test]
fn a_missing_file_keeps_the_defaults_and_says_why() {
    let (mut app, assets) = app_with(None, None);
    settle(&mut app, "speed", 1);
    assert_eq!(live(&app), 0.0);
    let e = entry(&app, "speed");
    assert_eq!((e.origin, e.failures), (ConfigOrigin::Defaults, 1));
    // Creating the file later is picked up.
    assets.write(SPEED, &speed(2.0));
    settle(&mut app, "speed", 2);
    assert_eq!(live(&app), 2.0);
}

#[test]
fn readers_see_a_change_through_change_detection() {
    #[derive(Resource, Default)]
    struct Seen(Vec<f32>);

    fn watch(config: Res<Config<Speed>>, mut seen: ResMut<Seen>) {
        if config.is_changed() {
            seen.0.push(config.metres_per_second);
        }
    }

    let (mut app, assets) = app_with(None, Some(&speed(1.0)));
    app.app_mut()
        .init_resource::<Seen>()
        .add_systems(Update, watch);
    settle(&mut app, "speed", 1);
    app.step(3);
    assets.write(SPEED, &speed(2.0));
    settle(&mut app, "speed", 2);
    app.step(3);
    // Defaults on the first frame, then each applied value exactly once.
    assert_eq!(app.world().resource::<Seen>().0, [0.0, 1.0, 2.0]);
}

#[test]
fn two_configs_sharing_the_ron_extension_never_mix() {
    let (mut app, assets) = app_with(None, Some(&speed(5.0)));
    assets.put(LABEL, r#"(text: "hello")"#);
    app.app_mut().add_config::<Label>(LABEL);
    settle(&mut app, "speed", 1);
    settle(&mut app, "label", 1);
    assert_eq!(live(&app), 5.0);
    assert_eq!(app.world().resource::<Config<Label>>().text, "hello");

    assets.write(LABEL, r#"(text: "changed")"#);
    settle(&mut app, "label", 2);
    assert_eq!(
        entry(&app, "speed").applied,
        1,
        "speed reloaded for a label edit"
    );
    assert_eq!(app.world().resource::<Config<Label>>().text, "changed");
}

#[test]
fn registering_the_same_config_twice_is_harmless() {
    let (mut app, _assets) = app_with(None, Some(&speed(1.0)));
    app.app_mut().add_config::<Speed>(SPEED);
    settle(&mut app, "speed", 1);
    assert_eq!(app.world().resource::<ConfigRegistry>().entries().len(), 1);
    app.step(5);
    assert_eq!(entry(&app, "speed").applied, 1);
}

#[test]
#[should_panic(expected = "registered from two paths")]
fn registering_one_config_from_two_paths_is_a_bug() {
    let (mut app, _assets) = app_with(None, None);
    app.app_mut().add_config::<Speed>("config/other.ron");
}

#[test]
fn hand_edits_apply_live_and_are_validated() {
    let (mut app, _assets) = app_with(None, Some(&speed(1.0)));
    settle(&mut app, "speed", 1);
    let world = app.world_mut();

    ConfigRegistry::apply_text(world, "speed", &speed(6.0)).unwrap();
    assert_eq!(world.resource::<Config<Speed>>().metres_per_second, 6.0);
    let e = world
        .resource::<ConfigRegistry>()
        .get("speed")
        .unwrap()
        .clone();
    assert_eq!((e.origin, e.applied), (ConfigOrigin::Edited, 2));

    let err = ConfigRegistry::apply_text(world, "speed", &speed(-1.0)).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid(_)));
    assert_eq!(world.resource::<Config<Speed>>().metres_per_second, 6.0);

    assert!(ConfigRegistry::apply_text(world, "nope", "()").is_err());
    let text = ConfigRegistry::current_text(world, "speed").unwrap();
    assert!(text.contains("6.0"), "{text}");
}

#[test]
fn saving_invalid_text_is_refused_and_changes_nothing() {
    let (mut app, _assets) = app_with(None, Some(&speed(1.0)));
    settle(&mut app, "speed", 1);
    // Validation runs before any write, so even a bogus folder is never touched.
    app.world_mut()
        .resource_mut::<ConfigRegistry>()
        .set_root(std::path::PathBuf::from("Z:/definitely/not/a/folder"));
    let result = ConfigRegistry::save_text(app.world_mut(), "speed", &speed(-5.0));
    assert!(result.is_err());
    assert_eq!(live(&app), 1.0, "invalid text was applied");
}

#[test]
fn a_burst_of_saves_ends_on_the_last_one() {
    let (mut app, assets) = app_with(None, Some(&speed(0.5)));
    settle(&mut app, "speed", 1);
    for i in 1..=25 {
        assets.write(SPEED, &speed(i as f32));
    }
    app.run_for(TIMEOUT, |w| {
        w.resource::<Config<Speed>>().metres_per_second == 25.0
    })
    .expect("the last save never became live");
    // Settle any stragglers, then the value must still be the last save.
    app.step(20);
    assert_eq!(live(&app), 25.0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Any sequence of good and bad saves leaves the last good value live, and
    /// counts every save exactly once as applied or rejected.
    #[test]
    fn the_live_value_is_always_the_last_good_save(
        saves in proptest::collection::vec(prop_oneof![
            (0.0f32..100.0).prop_map(Some),
            Just(None),
        ], 1..12),
        seed in any::<u64>(),
    ) {
        let (mut app, assets) = app_with(Some(seed), Some(&speed(1.0)));
        settle(&mut app, "speed", 1);
        let mut expected = 1.0;
        for (i, save) in saves.iter().enumerate() {
            match save {
                Some(v) => {
                    assets.write(SPEED, &speed(*v));
                    expected = *v;
                }
                None => assets.write(SPEED, "(metres_per_second: oops)"),
            }
            settle(&mut app, "speed", 2 + i as u32);
            prop_assert_eq!(live(&app), expected);
        }
        let e = entry(&app, "speed");
        let bad = saves.iter().filter(|s| s.is_none()).count() as u32;
        prop_assert_eq!(e.failures, bad);
        prop_assert_eq!(e.applied, 1 + saves.len() as u32 - bad);
    }
}
