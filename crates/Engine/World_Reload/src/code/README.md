# Code

## Purpose

This module handles what happens after new Rust code is swapped into the running game. Bevy points every system at its patched version by itself, but setup that ran once at startup never sees the new code. This module re-runs that setup after every code reload, removing what the old setup spawned first, so the game reflects the new code without a restart.

## Scope

- The engine's own code-reload message, written for each Bevy hot-patch notification when the hot-patch feature is on, and writable by tools and tests to take the same path without a patch.
- Registering setup systems that run at startup and again after each code reload.
- The marker for entities owned by reloadable setup, and their removal before setup runs again.
- A count of code reloads for diagnostics and panels.

## Out of scope

- Building and loading patches: the dx CLI and Bevy's hot-patch support, started through Xtask.
- Deciding what setup is reloadable: the crates that own the setup.
- Config files: the sibling `config` module.

## Relationships

Uses Bevy's ECS and, with the hot-patch feature, Bevy's hot-patch message. Used by World_Game, the app and any crate with reloadable setup.

## Rules

- Removal always finishes before setup runs again, under any system order.
- Several reload messages in one frame cause one rebuild.
- Entities without the marker are never touched.
- Everything works without the hot-patch feature, so it is testable in every build.
