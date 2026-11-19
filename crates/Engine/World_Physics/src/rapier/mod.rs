//! The only module that names Rapier.

use bevy::prelude::*;
use bevy_rapier3d::prelude::{NoUserData, RapierConfiguration, RapierPhysicsPlugin};
use world_reload::{Config, ConfigSystems};

use crate::tuning::PhysicsTuning;

pub(crate) fn build(app: &mut App) {
    app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default().in_fixed_schedule())
        .add_systems(PreUpdate, apply_tuning.after(ConfigSystems));
}

/// Pushes tuning into Rapier and the fixed clock whenever it changes, and into
/// any physics context created after the last change.
fn apply_tuning(
    tuning: Res<Config<PhysicsTuning>>,
    mut fixed: ResMut<Time<Fixed>>,
    mut contexts: Query<&mut RapierConfiguration>,
) {
    if tuning.is_changed() {
        fixed.set_timestep_hz(tuning.step_hz);
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

    use super::*;
    use crate::WorldPhysicsPlugin;
    use crate::tuning::PHYSICS_CONFIG_PATH;

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

    fn timestep(app: &HeadlessApp) -> Duration {
        app.world().resource::<Time<Fixed>>().timestep()
    }

    fn physics_app(initial: &str) -> (HeadlessApp, MemoryAssets) {
        let mut app = HeadlessApp::new();
        let assets = MemoryAssets::install(app.app_mut());
        assets.put(PHYSICS_CONFIG_PATH, initial);
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

        assets.write(PHYSICS_CONFIG_PATH, &tuning(30.0, -20.0));
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

        assets.write(PHYSICS_CONFIG_PATH, &tuning(1000.0, -9999.0));
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
