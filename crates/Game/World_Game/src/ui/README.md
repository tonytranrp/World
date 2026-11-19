# UI

## Purpose

The ui domain holds everything the player reads or clicks that is not the world: the HUD, menus and debug overlays. It is one of the few places where rasterized triangles are allowed, since the interface is drawn as ordinary Bevy UI on top of the traced frame.

## Scope

- HUD: crosshair, held-tool and ammunition display, health, targeted-material readout, interaction prompts.
- Menus: main menu, pause, settings screens, scenario and tool pickers that work with `sandbox` and `tools`.
- Debug overlays: frame time, brick-pool occupancy, active-body count, tier and backend, voxel counts, physics step cost, inspector panels for live tuning.
- UI state: which screens are open and how input focus is routed between UI and the player.
- Presenting settings changes; the settings values are stored by `app/`, and a backend change is shown as applying on restart.

## Out of scope

- Game rules and state changes belong in the domain that owns them; the UI sends requests as events.
- Rendering of the voxel world, depth and lighting belongs in World_Render.
- Reading and writing the settings file belongs in `app/`.
- Save file logic belongs in `save`.

## Relationships

Reads state from `player`, `tools`, `interaction` and `sandbox`, and diagnostics from the engine crates. Depends on Bevy UI and on the egui inspector for developer panels. Sends events to other domains.

## Rules

- UI is read-mostly: it displays state and emits events; it never mutates world or physics state directly.
- Developer overlays must be removable from release builds without affecting gameplay.
- UI must be legible at the 720p internal resolution used in the Mac floor mode.
- Depth from the traced world is available, so UI elements that live in the world occlude correctly.
- Systems stay in `Update` and avoid changing live component layouts.
