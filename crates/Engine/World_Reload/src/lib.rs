//! World_Reload: change the running game without restarting it.
//!
//! Two kinds of live change meet here:
//!
//! - **Data.** Tuning lives in RON files under `assets/config/`. Each file is a
//!   typed [`ConfigAsset`], loaded and hot-reloaded by the asset server,
//!   validated before it is applied, and mirrored into a [`Config<T>`] resource.
//!   A bad edit is rejected and reported; the last good value stays live.
//! - **Code.** With `dx serve --hot-patch`, Bevy patches systems in place. Setup
//!   systems ran once at startup, so they would never see the new code. They are
//!   registered with [`AppReloadExt::add_reloadable_systems`] and re-run after
//!   every [`CodeReloaded`], once the entities they spawned ([`ReloadScoped`])
//!   are gone.

mod code;
mod config;
mod plugin;
pub mod prelude;

pub use code::{AppReloadExt, CodeReloadStats, CodeReloaded, ReloadScoped, ReloadSystems};
pub use config::{
    AppConfigExt, Config, ConfigAsset, ConfigEntry, ConfigError, ConfigFile, ConfigOrigin,
    ConfigRegistry, ConfigSystems, SaveError, parse_config, to_pretty_ron,
};
pub use plugin::WorldReloadPlugin;
