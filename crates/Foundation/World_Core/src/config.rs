//! The contract for tunable data files under `assets/config/`.
//!
//! A type that implements [`ConfigAsset`] can be loaded, validated and
//! hot-reloaded by the Engine layer (`World_Reload`). The trait lives here,
//! with no Bevy, so Foundation types such as the material table can declare
//! themselves tunable in their own crate.

use serde::Serialize;
use serde::de::DeserializeOwned;

pub trait ConfigAsset:
    Serialize + DeserializeOwned + Clone + Default + Send + Sync + 'static
{
    /// Short unique name for logs and the developer panel, such as `"physics"`.
    const NAME: &'static str;

    /// Asset path of the file, relative to the asset root, such as
    /// `"config/physics.ron"`. Owned by the type so every registration agrees.
    const PATH: &'static str;

    /// Checks beyond what parsing enforces. List every problem, not just the
    /// first, so one reload reports every mistake. `Default` must pass.
    fn validate(&self) -> Result<(), Vec<String>> {
        Ok(())
    }
}
