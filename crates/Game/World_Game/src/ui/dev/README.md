# Developer panels

## Purpose

The in-game tools for tuning and iterating while the game runs. They show every config with its status, let people edit and save configs live, inspect entities, and re-run reloadable setup on demand. They exist only in development builds.

## Scope

- The entity inspector window.
- The config window: every registered config, where its value came from, applied and rejected counts, the last error, and an editor working on the file text so comments survive, with apply, save and revert.
- A hotkey and button that re-run reloadable setup, the same path a code hot-patch takes.
- Turning clicks into requests carried out by one exclusive system, so the behavior is testable without a window.

## Out of scope

- Loading, validating and saving configs: World_Reload.
- The config types and files: their owning crates and `assets/config`.
- Player-facing menus and the HUD: other parts of the ui domain.

## Relationships

Uses World_Reload's registry and messages, bevy_egui and bevy-inspector-egui. Nothing depends on it.

## Rules

- Built only with the `dev_tools` feature; release builds contain none of it.
- Windows are drawn only when a renderer exists; headless apps get request handling alone.
- Panels never change state directly: they send requests that go through the same validation as file reloads.
- Text that fails validation is never applied or written.
