//! The asset loader that turns a `.ron` file into a validated [`ConfigFile`].

use core::any::type_name;
use core::marker::PhantomData;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::reflect::TypePath;
use bevy::reflect::utility::GenericTypePathCell;

use super::parse::{ConfigError, parse_config_bytes};
use super::{ConfigAsset, ConfigFile};

/// Loads one config type. It registers no file extension: every config is a
/// `.ron`, and the asset server picks this loader by the requested asset type.
pub(crate) struct ConfigLoader<T>(PhantomData<fn() -> T>);

impl<T> Default for ConfigLoader<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T: ConfigAsset> TypePath for ConfigLoader<T> {
    fn type_path() -> &'static str {
        static CELL: GenericTypePathCell = GenericTypePathCell::new();
        CELL.get_or_insert::<Self, _>(|| {
            format!("world_reload::config::ConfigLoader<{}>", type_name::<T>())
        })
    }

    fn short_type_path() -> &'static str {
        static CELL: GenericTypePathCell = GenericTypePathCell::new();
        CELL.get_or_insert::<Self, _>(|| format!("ConfigLoader<{}>", T::NAME))
    }
}

impl<T: ConfigAsset> AssetLoader for ConfigLoader<T> {
    type Asset = ConfigFile<T>;
    type Settings = ();
    type Error = ConfigError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| ConfigError::Parse {
                line: 0,
                column: 0,
                message: format!("could not read the file: {e}"),
            })?;
        parse_config_bytes::<T>(&bytes).map(ConfigFile)
    }
}
