# World_Reload tests

## Purpose

Integration and property tests for live reloading, driven through a headless Bevy app. They prove that configs follow saves, that bad saves are rejected without effect, and that reloadable setup rebuilds cleanly after code reloads.

## Scope

- Config reload through an in-memory asset source whose watcher the test drives by hand: deterministic, no file system, no debounce.
- Config reload end to end through a real temporary folder and Bevy's real file watcher, including the save path used by the panels.
- Property tests over random sequences of good and bad saves, under randomized schedules.
- Code reload under every schedule seed, and the hot-patch bridge when built with the `hotpatch` feature.

## Out of scope

- Unit tests of parsing and the registry: inline in the source.
- Cross-crate checks of the shipped config files, the game's camera and shader hot reload: the root `tests/` suites.
- Running dx and patching real code: `cargo xtask hot --smoke`.

## Relationships

Uses World_Testkit_Bevy's headless app, asset sources and seeds.

## Rules

- Prefer the in-memory source; use the real folder only for what only the real path can show.
- Waits are bounded by a timeout and fail with the state they saw.
