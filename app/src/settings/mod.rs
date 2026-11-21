//! Resolved configuration: defaults, with command-line overrides on top.
//!
//! Persisting to and loading from a settings file builds on this structure;
//! every option has exactly one field here.

use world_render::prelude::Profile;

use crate::cli::{BackendChoice, Cli};

#[derive(Clone, Debug)]
pub struct Config {
    pub backend: BackendChoice,
    pub profile: Profile,
    pub list_adapters: bool,
    pub exit_after_frames: Option<u32>,
    pub screenshot: Option<std::path::PathBuf>,
    pub frame_times: Option<std::path::PathBuf>,
    pub walk: Option<f32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            backend: BackendChoice::Auto,
            profile: Profile::Full,
            list_adapters: false,
            exit_after_frames: None,
            screenshot: None,
            frame_times: None,
            walk: None,
        }
    }
}

impl Config {
    /// Defaults with the command-line overrides applied.
    pub fn from_cli(cli: &Cli) -> Self {
        Self {
            backend: cli.backend,
            profile: if cli.mac_floor {
                Profile::MacFloor
            } else {
                Profile::Full
            },
            list_adapters: cli.list_adapters,
            exit_after_frames: cli.exit_after_frames,
            screenshot: cli.screenshot.clone(),
            frame_times: cli.frame_times.clone(),
            walk: cli.walk,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn mac_floor_flag_selects_the_profile() {
        let cli = Cli::try_parse_from(["world", "--mac-floor"]).unwrap();
        assert_eq!(Config::from_cli(&cli).profile, Profile::MacFloor);
    }

    #[test]
    fn no_flags_gives_defaults() {
        let cli = Cli::try_parse_from(["world"]).unwrap();
        let config = Config::from_cli(&cli);
        assert_eq!(config.profile, Profile::Full);
        assert_eq!(config.backend, BackendChoice::Auto);
    }
}
