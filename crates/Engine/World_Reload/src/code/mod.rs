//! Code reload: what happens after `dx serve --hot-patch` swaps in new code.
//!
//! Bevy re-points every system at its patched version on its own. What it
//! cannot do is re-run setup: a `Startup` system that spawned the camera ran
//! once, with the old code. Setup registered here runs at startup and again
//! after each [`CodeReloaded`]: first every [`ReloadScoped`] entity is
//! despawned ([`ReloadSystems::Teardown`]), then the setup systems run again
//! ([`ReloadSystems::Rebuild`]).
//!
//! [`CodeReloaded`] is this crate's own message. With the `hotpatch` feature it
//! is written for each Bevy `HotPatched`; tests and the developer panel write
//! it directly to exercise the same path without a patch.

use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::ScheduleSystem;
use bevy::prelude::*;

/// New code is live. Reloadable setup re-runs in the next `PreUpdate`.
#[derive(Message, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeReloaded;

/// Marks an entity spawned by reloadable setup. It is despawned, with its
/// children, before the setup runs again.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ReloadScoped;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeReloadStats {
    /// Code reloads handled since startup.
    pub reloads: u32,
}

/// The two steps that follow a [`CodeReloaded`], both in `PreUpdate`.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReloadSystems {
    /// Despawns every [`ReloadScoped`] entity.
    Teardown,
    /// Re-runs the setup registered with [`AppReloadExt::add_reloadable_systems`].
    Rebuild,
}

pub trait AppReloadExt {
    /// Runs `make()`'s systems at startup and again after every code reload.
    /// `make` is called twice, once per schedule. The systems must tag what
    /// they spawn with [`ReloadScoped`] so the old copies are removed first.
    fn add_reloadable_systems<M, S>(&mut self, make: impl Fn() -> S) -> &mut Self
    where
        S: IntoScheduleConfigs<ScheduleSystem, M>;
}

impl AppReloadExt for App {
    fn add_reloadable_systems<M, S>(&mut self, make: impl Fn() -> S) -> &mut Self
    where
        S: IntoScheduleConfigs<ScheduleSystem, M>,
    {
        let rebuild: ScheduleConfigs<ScheduleSystem> =
            make().into_configs().in_set(ReloadSystems::Rebuild);
        self.add_systems(Startup, make())
            .add_systems(PreUpdate, rebuild)
    }
}

pub(crate) fn build(app: &mut App) {
    app.add_message::<CodeReloaded>()
        .init_resource::<CodeReloadStats>()
        .configure_sets(
            PreUpdate,
            (
                ReloadSystems::Teardown.run_if(on_message::<CodeReloaded>),
                ReloadSystems::Rebuild
                    .after(ReloadSystems::Teardown)
                    .run_if(on_message::<CodeReloaded>),
            ),
        )
        .add_systems(
            PreUpdate,
            (count_reload, despawn_scoped).in_set(ReloadSystems::Teardown),
        );

    // Bevy's `HotPatchPlugin` registers `HotPatched`, but apps without it
    // (headless tools, tests) must not fail; registering twice is harmless.
    #[cfg(feature = "hotpatch")]
    app.add_message::<bevy::ecs::HotPatched>()
        .add_systems(First, forward_hot_patches);
}

fn count_reload(mut stats: ResMut<CodeReloadStats>) {
    stats.reloads += 1;
    info!(
        "code reload #{}: re-running reloadable setup",
        stats.reloads
    );
}

fn despawn_scoped(mut commands: Commands, scoped: Query<Entity, With<ReloadScoped>>) {
    for entity in &scoped {
        commands.entity(entity).despawn();
    }
}

/// Bevy writes `HotPatched` in `Last`; it is forwarded in the next `First`, so
/// the reload runs in the following `PreUpdate`.
#[cfg(feature = "hotpatch")]
fn forward_hot_patches(
    mut patches: MessageReader<bevy::ecs::HotPatched>,
    mut reloaded: MessageWriter<CodeReloaded>,
) {
    if patches.read().count() > 0 {
        reloaded.write(CodeReloaded);
    }
}
