# Test asset sources

## Purpose

Asset roots for tests of hot reloading. One lives in memory and its watcher is driven by the test, so changing a file and announcing the change are a single deterministic step. The other is a real temporary folder watched by Bevy's own file watcher, for end-to-end checks of the real path.

## Scope

- An in-memory default asset source with a hand-driven watcher: store files, change them with an announcement, delete them.
- A temporary folder used as the asset root with watching on, removed when the test ends.

## Out of scope

- What is being reloaded and how it is validated: the crates under test.
- Stepping the app and waiting for conditions: the `app` module.

## Relationships

Used by World_Reload, World_Physics, World_Game and the root `tests/` suites as a dev-dependency.

## Rules

- The in-memory source is the default choice; the real folder is for what only the real watcher can show.
- Temporary folders are unique per test and process, and are cleaned up.
