# Crates

## Purpose

This folder holds every library crate of the engine and game, grouped into three category folders: `Foundation`, `Engine` and `Game`. The grouping encodes the dependency direction of the whole project, so a crate's location tells you what it may depend on and who may depend on it. The binary lives in `app/` and the offline tooling in `tools/`, both outside this folder.

## Scope

- `Foundation` holds crates with no Bevy dependency: the voxel core (64-tree, palette, edits, serialization), procedural generation, and the CPU reference ray tracer. They test fast, survive Bevy upgrades unchanged, and can run headless.
- `Engine` holds the Bevy crates that are stable and rarely edited: rendering, physics, animation and simulation. They change at the pace of the engine design, not the pace of gameplay iteration.
- `Game` holds the gameplay crate, the fast-changing layer that is hot-patched during development.
- New crates are added here only if they are libraries that other crates compile against.

## Out of scope

- The executable, backend selection and settings belong in `app/`.
- Build automation, baking and conversion utilities belong in `tools/`.
- Runtime assets (shaders, material tables, worldgen parameters) belong in the top-level assets folder, not inside crates, except for WESL modules owned by the render crate.
- Design documents belong in `docs/`.

## Relationships

Dependencies point one way only: Foundation <- Engine <- Game <- app. A Foundation crate may depend on other Foundation crates but never on Engine or Game. An Engine crate may depend on Foundation and, where unavoidable, on sibling Engine crates through their public plugin or prelude. The Game crate may depend on Foundation and Engine. Only `app/` assembles all of them. Tools may depend on Foundation crates to share formats and constants, never on Engine or Game.

## Rules

- Adding a crate: create it at `crates/<Category>/<Pascal_Name>`. The workspace member glob picks it up automatically, so no workspace edit is needed.
- Naming: crate folders are Pascal_Case with underscores (for example World_Core). Package names are the same name in snake_case (world_core). Module folders inside a crate's source tree are snake_case.
- Every crate exposes one narrow public surface. Each Bevy crate exposes a single Plugin that registers everything it owns, and a prelude re-exporting the types other crates are meant to use. Internal modules stay private.
- Foundation crates must not name Bevy, wgpu or any physics engine in their dependency list.
- The physics engine is confined to the physics crate, so replacing it touches one crate.
- The CPU 64-tree in World_Core is the single source of truth. Other crates mirror it and never keep a competing copy of world state.
- Every folder inside a crate has its own README describing what belongs there. Read it before adding files.
