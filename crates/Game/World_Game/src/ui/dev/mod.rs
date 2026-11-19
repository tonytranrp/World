//! Developer panels for live tuning, built only with the `dev_tools` feature.
//!
//! - **F1** entity inspector (bevy-inspector-egui).
//! - **F2** config editor: every registered config with its status and last
//!   error. Edit the text, then **Apply** (live, not saved) or **Save** (write
//!   the file; the hot reload applies it). Text that fails validation is never
//!   applied or written.
//! - **F5** re-runs reloadable setup, the same path a code hot-patch takes.
//!
//! The egui system only records what was clicked ([`PanelRequest`]); an
//! exclusive system carries it out, so the logic is testable without a window.

mod panel;
mod requests;

use bevy::input::common_conditions::input_toggle_active;
use bevy::prelude::*;
use bevy_egui::{EguiPlugin, EguiPrimaryContextPass};
use bevy_inspector_egui::quick::WorldInspectorPlugin;

pub(crate) use requests::{ConfigPanel, PanelRequest, PanelRequests};

pub(crate) struct DevToolsPlugin;

impl Plugin for DevToolsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConfigPanel>()
            .init_resource::<PanelRequests>()
            .add_systems(Update, (requests::hotkeys, requests::carry_out).chain());
        // The windows need a renderer; headless apps (tests, servers) get the
        // request handling only.
        if !app.is_plugin_added::<bevy::render::RenderPlugin>() {
            return;
        }
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin::default());
        }
        app.add_plugins(
            WorldInspectorPlugin::new().run_if(input_toggle_active(false, KeyCode::F1)),
        )
        .add_systems(EguiPrimaryContextPass, panel::config_panel);
    }
}
