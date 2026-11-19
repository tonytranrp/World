//! Typed, validated, hot-reloaded configs.
//!
//! Lifecycle of one config `T` registered at `config/x.ron`:
//!
//! 1. `Config<T>` holds `T::default()` from the start, so readers never wait.
//! 2. At startup the file is loaded through [`ConfigLoader`], which parses and
//!    validates. Success replaces the value; failure keeps the defaults and is
//!    recorded in the [`ConfigRegistry`].
//! 3. With a file watcher on, each save reloads the file the same way. A bad
//!    save is rejected and the last good value stays live.
//! 4. The developer panel may apply or save text through the registry.

mod loader;
mod parse;
mod registry;

use core::any::type_name;
use std::ops::Deref;

use bevy::asset::io::file::FileAssetReader;
use bevy::asset::{AssetLoadFailedEvent, AssetPlugin};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::reflect::utility::GenericTypePathCell;
use loader::ConfigLoader;
pub use parse::{ConfigError, parse_config, to_pretty_ron};
pub use registry::{ConfigEntry, ConfigOrigin, ConfigRegistry, SaveError};
/// The contract a tunable type implements. Defined in `World_Core` so
/// Foundation types can implement it without Bevy.
pub use world_core::ConfigAsset;

/// The live value of a config. Systems read it like any resource and can use
/// change detection to react to reloads.
#[derive(Resource, Debug, Default)]
pub struct Config<T: ConfigAsset>(pub T);

impl<T: ConfigAsset> Deref for Config<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// A config as an asset: what the loader produces from one file.
#[derive(Asset, Debug)]
pub struct ConfigFile<T: ConfigAsset>(pub T);

impl<T: ConfigAsset> TypePath for ConfigFile<T> {
    fn type_path() -> &'static str {
        static CELL: GenericTypePathCell = GenericTypePathCell::new();
        CELL.get_or_insert::<Self, _>(|| {
            format!("world_reload::config::ConfigFile<{}>", type_name::<T>())
        })
    }

    fn short_type_path() -> &'static str {
        static CELL: GenericTypePathCell = GenericTypePathCell::new();
        CELL.get_or_insert::<Self, _>(|| format!("ConfigFile<{}>", T::NAME))
    }
}

#[derive(Resource)]
struct ConfigHandle<T: ConfigAsset> {
    path: String,
    handle: Option<Handle<ConfigFile<T>>>,
}

/// Configs are applied in `PreUpdate`, so `Update` systems see a reload in the
/// same frame it lands.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConfigSystems;

pub trait AppConfigExt {
    /// Registers config `T`, loaded from `path` under the asset root. Safe to
    /// call from several plugins: the second call with the same path is a no-op.
    fn add_config<T: ConfigAsset>(&mut self, path: &str) -> &mut Self;
}

impl AppConfigExt for App {
    fn add_config<T: ConfigAsset>(&mut self, path: &str) -> &mut Self {
        if !self.world().contains_resource::<ConfigRegistry>() {
            self.init_resource::<ConfigRegistry>();
        }
        let first = self
            .world_mut()
            .resource_mut::<ConfigRegistry>()
            .register::<T>(path);
        if !first {
            return self;
        }
        self.init_asset::<ConfigFile<T>>()
            .register_asset_loader(ConfigLoader::<T>::default())
            .insert_resource(Config::<T>::default())
            .insert_resource(ConfigHandle::<T> {
                path: path.to_owned(),
                handle: None,
            })
            .add_systems(Startup, start_loading::<T>)
            .add_systems(
                PreUpdate,
                (apply_loaded::<T>, record_failures::<T>).in_set(ConfigSystems),
            )
    }
}

/// Records the asset root so configs can be saved back to disk.
pub(crate) fn build(app: &mut App) {
    if !app.world().contains_resource::<ConfigRegistry>() {
        app.init_resource::<ConfigRegistry>();
    }
    let folder = app
        .get_added_plugins::<AssetPlugin>()
        .first()
        .map(|plugin| plugin.file_path.clone());
    if let Some(folder) = folder {
        let root = FileAssetReader::get_base_path().join(folder);
        app.world_mut()
            .resource_mut::<ConfigRegistry>()
            .set_root(root);
    }
}

fn start_loading<T: ConfigAsset>(server: Res<AssetServer>, mut config: ResMut<ConfigHandle<T>>) {
    let handle = server.load::<ConfigFile<T>>(config.path.clone());
    config.handle = Some(handle);
}

fn apply_loaded<T: ConfigAsset>(
    mut events: MessageReader<AssetEvent<ConfigFile<T>>>,
    handle: Res<ConfigHandle<T>>,
    files: Res<Assets<ConfigFile<T>>>,
    mut config: ResMut<Config<T>>,
    mut registry: ResMut<ConfigRegistry>,
) {
    let Some(id) = handle.handle.as_ref().map(Handle::id) else {
        return;
    };
    // Several events can arrive in one frame; apply the newest value once.
    let changed = events.read().any(|event| match event {
        AssetEvent::Added { id: e } | AssetEvent::Modified { id: e } => *e == id,
        _ => false,
    });
    if !changed {
        return;
    }
    if let Some(file) = files.get(id) {
        config.0 = file.0.clone();
        registry.record_applied(T::NAME, ConfigOrigin::File);
        let applied = registry.get(T::NAME).map_or(0, |e| e.applied);
        info!("config {}: applied {} (#{applied})", T::NAME, handle.path);
    }
}

fn record_failures<T: ConfigAsset>(
    mut events: MessageReader<AssetLoadFailedEvent<ConfigFile<T>>>,
    mut registry: ResMut<ConfigRegistry>,
) {
    for event in events.read() {
        let error = event.error.to_string();
        warn!(
            "config {}: rejected {}, keeping the last good value: {error}",
            T::NAME,
            event.path
        );
        registry.record_failure(T::NAME, error);
    }
}
