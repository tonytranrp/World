//! The only module that names Rapier.

use bevy::prelude::*;
use bevy_rapier3d::prelude::{NoUserData, RapierConfiguration, RapierPhysicsPlugin, TimestepMode};
use world_reload::{Config, ConfigSystems};

use crate::tuning::PhysicsTuning;

pub(crate) fn build(app: &mut App) {
    // Rapier's default `Variable { max_dt: 1/60 }` would cap every fixed step at
    // 1/60 s, so a 30 Hz `step_hz` would run the world at half speed. Inserted
    // before the plugin, which only initialises it when missing.
    app.insert_resource(fixed_step(PhysicsTuning::default().step_hz))
        .add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule())
        .add_systems(PreUpdate, apply_tuning.after(ConfigSystems));
}

/// Rapier integrates exactly one fixed step of `1 / step_hz` seconds per tick.
fn fixed_step(step_hz: f64) -> TimestepMode {
    TimestepMode::Fixed {
        dt: (1.0 / step_hz) as f32,
        substeps: 1,
    }
}

/// Pushes tuning into Rapier and the fixed clock whenever it changes, and into
/// any physics context created after the last change. The fixed clock and
/// Rapier's step always change together, so physics runs at real-time speed.
fn apply_tuning(
    tuning: Res<Config<PhysicsTuning>>,
    mut fixed: ResMut<Time<Fixed>>,
    mut mode: ResMut<TimestepMode>,
    mut contexts: Query<&mut RapierConfiguration>,
) {
    if tuning.is_changed() {
        fixed.set_timestep_hz(tuning.step_hz);
        *mode = fixed_step(tuning.step_hz);
    }
    let (x, y, z) = tuning.gravity_m_s2;
    for mut context in &mut contexts {
        if tuning.is_changed() || context.is_added() {
            context.gravity = Vec3::new(x, y, z);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use world_reload::WorldReloadPlugin;
    use world_testkit_bevy::app::HeadlessApp;
    use world_testkit_bevy::assets::MemoryAssets;

    use world_core::ConfigAsset;

    use super::*;
    use crate::WorldPhysicsPlugin;

    fn tuning(step_hz: f64, gravity_y: f32) -> String {
        let t = PhysicsTuning {
            step_hz,
            gravity_m_s2: (0.0, gravity_y, 0.0),
            ..PhysicsTuning::default()
        };
        world_reload::to_pretty_ron(&t)
    }

    fn gravity(app: &mut HeadlessApp) -> Vec<Vec3> {
        app.world_mut()
            .query::<&RapierConfiguration>()
            .iter(app.world())
            .map(|c| c.gravity)
            .collect()
    }

    /// The fixed clock's step, after checking Rapier integrates the same step.
    fn timestep(app: &HeadlessApp) -> Duration {
        let clock = app.world().resource::<Time<Fixed>>().timestep();
        match *app.world().resource::<TimestepMode>() {
            TimestepMode::Fixed { dt, substeps: 1 } => {
                assert!(
                    (f64::from(dt) - clock.as_secs_f64()).abs() < 1e-6,
                    "Rapier steps {dt} s, the clock {clock:?}"
                );
            }
            other => panic!("Rapier is not stepping a fixed dt: {other:?}"),
        }
        clock
    }

    fn physics_app(initial: &str) -> (HeadlessApp, MemoryAssets) {
        let mut app = HeadlessApp::new();
        let assets = MemoryAssets::install(app.app_mut());
        assets.put(PhysicsTuning::PATH, initial);
        app.add_plugins((TransformPlugin, WorldReloadPlugin, WorldPhysicsPlugin));
        (app, assets)
    }

    #[test]
    fn tuning_reaches_rapier_and_the_fixed_clock_and_follows_reloads() {
        let (mut app, assets) = physics_app(&tuning(120.0, -3.0));
        app.run_for(Duration::from_secs(10), |w| {
            w.resource::<Config<PhysicsTuning>>().step_hz == 120.0
        })
        .expect("physics tuning never loaded");
        app.step(2);
        let g = gravity(&mut app);
        assert!(!g.is_empty(), "no Rapier context exists");
        assert!(g.iter().all(|g| *g == Vec3::new(0.0, -3.0, 0.0)), "{g:?}");
        assert_eq!(timestep(&app), Duration::from_secs_f64(1.0 / 120.0));

        assets.write(PhysicsTuning::PATH, &tuning(30.0, -20.0));
        app.run_for(Duration::from_secs(10), |w| {
            w.resource::<Config<PhysicsTuning>>().step_hz == 30.0
        })
        .expect("the reload never landed");
        app.step(1);
        assert!(
            gravity(&mut app)
                .iter()
                .all(|g| *g == Vec3::new(0.0, -20.0, 0.0))
        );
        assert_eq!(timestep(&app), Duration::from_secs_f64(1.0 / 30.0));
    }

    #[test]
    fn a_rejected_reload_leaves_rapier_untouched() {
        let (mut app, assets) = physics_app(&tuning(60.0, -9.0));
        app.run_for(Duration::from_secs(10), |w| {
            w.resource::<Config<PhysicsTuning>>().gravity_m_s2.1 == -9.0
        })
        .expect("physics tuning never loaded");

        assets.write(PhysicsTuning::PATH, &tuning(1000.0, -9999.0));
        app.run_for(Duration::from_secs(10), |w| {
            w.resource::<world_reload::ConfigRegistry>()
                .get("physics")
                .is_some_and(|e| e.failures == 1)
        })
        .expect("the bad tuning was never rejected");
        app.step(2);
        assert!(
            gravity(&mut app)
                .iter()
                .all(|g| *g == Vec3::new(0.0, -9.0, 0.0))
        );
        assert_eq!(timestep(&app), Duration::from_secs_f64(1.0 / 60.0));
    }
}
