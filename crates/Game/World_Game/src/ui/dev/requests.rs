//! What the panels ask for, and the exclusive system that does it.

use bevy::prelude::*;
use world_reload::{CodeReloaded, ConfigRegistry};

/// One click in the developer panels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PanelRequest {
    /// Select a config and load its file text into the editor.
    Open(&'static str),
    /// Apply editor text to the live config without saving.
    Apply { name: &'static str, text: String },
    /// Apply editor text and write it to the config's file.
    Save { name: &'static str, text: String },
    /// Throw away edits: reload the file into the editor and the game.
    RevertToFile(&'static str),
    /// Re-run reloadable setup, as a code hot-patch does.
    RerunSetup,
}

#[derive(Resource, Default, Debug)]
pub(crate) struct PanelRequests(pub Vec<PanelRequest>);

/// Editor state of the config panel.
#[derive(Resource, Default, Debug)]
pub(crate) struct ConfigPanel {
    pub open: bool,
    pub selected: Option<&'static str>,
    pub text: String,
    /// Outcome of the last action: `Ok` message or `Err` message.
    pub message: Option<Result<String, String>>,
}

pub(crate) fn hotkeys(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    egui: Option<Res<bevy_egui::input::EguiWantsInput>>,
    mut panel: ResMut<ConfigPanel>,
    mut requests: ResMut<PanelRequests>,
) {
    let Some(keys) = keys else {
        return;
    };
    // Typing in the config editor must not toggle panels or rebuild the game.
    if egui.is_some_and(|e| e.wants_any_keyboard_input()) {
        return;
    }
    if keys.just_pressed(KeyCode::F2) {
        panel.open = !panel.open;
    }
    if keys.just_pressed(KeyCode::F5) {
        requests.0.push(PanelRequest::RerunSetup);
    }
}

/// The text to edit: the file itself, so comments survive, or the live value
/// when there is no readable file.
fn editor_text(world: &World, name: &str) -> String {
    let file = world.resource::<ConfigRegistry>().file_path(name);
    file.and_then(|path| std::fs::read_to_string(path).ok())
        .or_else(|| ConfigRegistry::current_text(world, name))
        .unwrap_or_default()
}

pub(crate) fn carry_out(world: &mut World) {
    let requests = std::mem::take(&mut world.resource_mut::<PanelRequests>().0);
    for request in requests {
        let message = match request {
            // Clicking the config already open keeps any unsaved editor text.
            PanelRequest::Open(name) if world.resource::<ConfigPanel>().selected == Some(name) => {
                None
            }
            PanelRequest::Open(name) => {
                let text = editor_text(world, name);
                let mut panel = world.resource_mut::<ConfigPanel>();
                panel.selected = Some(name);
                panel.text = text;
                None
            }
            PanelRequest::Apply { name, text } => Some(
                ConfigRegistry::apply_text(world, name, &text)
                    .map(|()| format!("{name}: applied (not saved)"))
                    .map_err(|e| format!("{name}: rejected, nothing changed: {e}")),
            ),
            PanelRequest::Save { name, text } => Some(
                ConfigRegistry::save_text(world, name, &text)
                    .map(|path| format!("{name}: saved to {}", path.display()))
                    .map_err(|e| format!("{name}: {e}")),
            ),
            PanelRequest::RevertToFile(name) => {
                let path = world
                    .resource::<ConfigRegistry>()
                    .get(name)
                    .map(|e| e.path.clone());
                if let (Some(path), Some(server)) = (path, world.get_resource::<AssetServer>()) {
                    server.reload(path);
                }
                let text = editor_text(world, name);
                world.resource_mut::<ConfigPanel>().text = text;
                Some(Ok(format!("{name}: reloading from file")))
            }
            PanelRequest::RerunSetup => {
                world.write_message(CodeReloaded);
                Some(Ok("re-running reloadable setup".into()))
            }
        };
        if let Some(message) = message {
            world.resource_mut::<ConfigPanel>().message = Some(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde::{Deserialize, Serialize};
    use world_core::ConfigAsset;
    use world_reload::{AppConfigExt, CodeReloadStats, Config, ConfigOrigin, WorldReloadPlugin};
    use world_testkit_bevy::app::HeadlessApp;
    use world_testkit_bevy::assets::DiskAssets;

    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fov {
        degrees: f32,
    }

    impl Default for Fov {
        fn default() -> Self {
            Self { degrees: 60.0 }
        }
    }

    impl ConfigAsset for Fov {
        const NAME: &'static str = "fov";
        const PATH: &'static str = PATH;
        fn validate(&self) -> Result<(), Vec<String>> {
            if (30.0..=120.0).contains(&self.degrees) {
                Ok(())
            } else {
                Err(vec!["degrees must be 30..=120".into()])
            }
        }
    }

    const PATH: &str = "config/fov.ron";
    const FILE: &str = "// field of view\n(degrees: 70.0)\n";
    const WAIT: Duration = Duration::from_secs(20);

    fn app(disk: &DiskAssets) -> HeadlessApp {
        disk.write(PATH, FILE);
        let mut app = HeadlessApp::new();
        app.add_plugins(disk.plugin())
            .add_plugins(WorldReloadPlugin)
            .add_plugins(super::super::DevToolsPlugin);
        app.app_mut().add_config::<Fov>();
        app.run_for(WAIT, |w| w.resource::<Config<Fov>>().degrees == 70.0)
            .expect("the config never loaded");
        app
    }

    fn send(app: &mut HeadlessApp, request: PanelRequest) -> Option<Result<String, String>> {
        app.world_mut()
            .resource_mut::<PanelRequests>()
            .0
            .push(request);
        app.step(1);
        app.world().resource::<ConfigPanel>().message.clone()
    }

    fn degrees(app: &HeadlessApp) -> f32 {
        app.world().resource::<Config<Fov>>().degrees
    }

    fn origin(app: &HeadlessApp) -> ConfigOrigin {
        app.world()
            .resource::<ConfigRegistry>()
            .get("fov")
            .unwrap()
            .origin
    }

    #[test]
    fn opening_a_config_loads_the_file_text_with_its_comments() {
        let disk = DiskAssets::new("panel_open");
        let mut app = app(&disk);
        send(&mut app, PanelRequest::Open("fov"));
        let panel = app.world().resource::<ConfigPanel>();
        assert_eq!(panel.selected, Some("fov"));
        assert_eq!(panel.text, FILE);
    }

    #[test]
    fn apply_changes_the_live_value_but_not_the_file() {
        let disk = DiskAssets::new("panel_apply");
        let mut app = app(&disk);
        let text = "(degrees: 90.0)".to_string();
        let msg = send(&mut app, PanelRequest::Apply { name: "fov", text });
        assert!(matches!(msg, Some(Ok(_))), "{msg:?}");
        assert_eq!(degrees(&app), 90.0);
        assert_eq!(disk.read(PATH), FILE);
        assert_eq!(origin(&app), ConfigOrigin::Edited);
    }

    #[test]
    fn an_invalid_apply_or_save_changes_nothing_and_says_why() {
        let disk = DiskAssets::new("panel_invalid");
        let mut app = app(&disk);
        for save in [false, true] {
            let text = "(degrees: 500.0)".to_string();
            let request = if save {
                PanelRequest::Save { name: "fov", text }
            } else {
                PanelRequest::Apply { name: "fov", text }
            };
            let msg = send(&mut app, request);
            assert!(
                matches!(&msg, Some(Err(e)) if e.contains("degrees")),
                "{msg:?}"
            );
        }
        assert_eq!(degrees(&app), 70.0);
        assert_eq!(disk.read(PATH), FILE);
    }

    #[test]
    fn save_writes_the_editor_text_and_the_reload_confirms_it() {
        let disk = DiskAssets::new("panel_save");
        let mut app = app(&disk);
        let text = "// field of view\n(degrees: 100.0)\n".to_string();
        let msg = send(
            &mut app,
            PanelRequest::Save {
                name: "fov",
                text: text.clone(),
            },
        );
        assert!(matches!(msg, Some(Ok(_))), "{msg:?}");
        assert_eq!(disk.read(PATH), text);
        app.run_for(WAIT, |w| {
            w.resource::<ConfigRegistry>().get("fov").unwrap().origin == ConfigOrigin::File
        })
        .expect("the saved file was never reloaded");
        assert_eq!(degrees(&app), 100.0);
    }

    #[test]
    fn revert_throws_away_a_hand_edit() {
        let disk = DiskAssets::new("panel_revert");
        let mut app = app(&disk);
        let text = "(degrees: 45.0)".to_string();
        send(&mut app, PanelRequest::Apply { name: "fov", text });
        assert_eq!(degrees(&app), 45.0);
        send(&mut app, PanelRequest::RevertToFile("fov"));
        app.run_for(WAIT, |w| w.resource::<Config<Fov>>().degrees == 70.0)
            .expect("revert never restored the file value");
        assert_eq!(app.world().resource::<ConfigPanel>().text, FILE);
    }

    #[test]
    fn rerun_setup_takes_the_code_reload_path() {
        let disk = DiskAssets::new("panel_rerun");
        let mut app = app(&disk);
        send(&mut app, PanelRequest::RerunSetup);
        app.step(1);
        assert_eq!(app.world().resource::<CodeReloadStats>().reloads, 1);
    }
}
