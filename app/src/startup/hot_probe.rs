//! A canary for Rust hot-patching, built only with the `hotpatch` feature.
//!
//! It logs `hot probe: <value>` at startup and again after every code reload.
//! `cargo xtask hot --smoke` edits [`probe_value`] while the game runs and
//! waits for the new value in the log, which proves end to end that a patch
//! was built, loaded and picked up by reloadable setup.

use bevy::prelude::*;
use world_reload::AppReloadExt;

pub(super) fn build(app: &mut App) {
    app.add_reloadable_systems(|| log_probe);
}

/// The smoke test rewrites the string on the next line. Keep it on one line.
fn probe_value() -> &'static str {
    "alpha"
}

fn log_probe() {
    info!("hot probe: {}", probe_value());
}
