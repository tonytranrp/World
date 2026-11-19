//! The F2 config editor window. Draws state and records clicks; nothing else.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use world_reload::{CodeReloadStats, ConfigOrigin, ConfigRegistry};

use super::{ConfigPanel, PanelRequest, PanelRequests};

const ERROR: egui::Color32 = egui::Color32::from_rgb(235, 90, 80);
const OK: egui::Color32 = egui::Color32::from_rgb(110, 200, 120);

pub(super) fn config_panel(
    mut contexts: EguiContexts,
    registry: Option<Res<ConfigRegistry>>,
    stats: Option<Res<CodeReloadStats>>,
    mut panel: ResMut<ConfigPanel>,
    mut requests: ResMut<PanelRequests>,
) -> Result {
    if !panel.open {
        return Ok(());
    }
    // No context this frame (the window is closing, or none yet): skip, since
    // a returned error would go to Bevy's default handler, which panics.
    let Ok(ctx) = contexts.ctx_mut() else {
        return Ok(());
    };
    let mut open = true;
    egui::Window::new("Configs (F2)")
        .open(&mut open)
        .default_width(520.0)
        .show(ctx, |ui| {
            if let Some(registry) = &registry {
                entry_list(ui, registry, &panel, &mut requests);
            } else {
                ui.label("No configs are registered.");
            }
            ui.separator();
            if let Some(name) = panel.selected {
                editor(ui, name, registry.as_deref(), &mut panel, &mut requests);
            }
            ui.separator();
            ui.horizontal(|ui| {
                let reloads = stats.as_ref().map_or(0, |s| s.reloads);
                ui.label(format!("Code reloads: {reloads}"));
                if ui.button("Re-run setup (F5)").clicked() {
                    requests.0.push(PanelRequest::RerunSetup);
                }
            });
        });
    panel.open = open;
    Ok(())
}

fn entry_list(
    ui: &mut egui::Ui,
    registry: &ConfigRegistry,
    panel: &ConfigPanel,
    requests: &mut PanelRequests,
) {
    for entry in registry.entries() {
        ui.horizontal(|ui| {
            let selected = panel.selected == Some(entry.name);
            if ui.selectable_label(selected, entry.name).clicked() {
                requests.0.push(PanelRequest::Open(entry.name));
            }
            let origin = match entry.origin {
                ConfigOrigin::Defaults => "defaults",
                ConfigOrigin::File => "file",
                ConfigOrigin::Edited => "edited, unsaved",
            };
            ui.label(format!(
                "{origin} | applied {} | rejected {}",
                entry.applied, entry.failures
            ));
        });
        if let Some(error) = &entry.last_error {
            ui.colored_label(ERROR, format!("  {error}"));
        }
    }
}

fn editor(
    ui: &mut egui::Ui,
    name: &'static str,
    registry: Option<&ConfigRegistry>,
    panel: &mut ConfigPanel,
    requests: &mut PanelRequests,
) {
    if let Some(path) = registry.and_then(|r| r.file_path(name)) {
        ui.small(path.display().to_string());
    }
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut panel.text)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(16),
            );
        });
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            requests.0.push(PanelRequest::Apply {
                name,
                text: panel.text.clone(),
            });
        }
        if ui.button("Save").clicked() {
            requests.0.push(PanelRequest::Save {
                name,
                text: panel.text.clone(),
            });
        }
        if ui.button("Revert to file").clicked() {
            requests.0.push(PanelRequest::RevertToFile(name));
        }
    });
    match &panel.message {
        Some(Ok(text)) => {
            ui.colored_label(OK, text);
        }
        Some(Err(text)) => {
            ui.colored_label(ERROR, text);
        }
        None => {}
    }
}
