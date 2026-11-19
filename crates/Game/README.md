# Game

## Purpose

This category folder holds the crates that make up the game itself, as opposed to the reusable engine beneath it. The game is a single-player, first-person destruction sandbox for Windows and macOS: the player walks through a large fixed voxel world and cuts, digs, blasts and demolishes it, watching disconnected pieces fall as rigid bodies. Everything specific to that experience lives here.

## Scope

- Gameplay crates that consume Foundation and Engine crates through their plugins and preludes.
- Systems whose design changes often: player control, tools, targeting and damage, sandbox scenarios, creature behavior, gameplay-side effects, audio triggers, user interface and save orchestration.
- Game-level data types and components that only gameplay cares about, such as tool definitions, inventory state and scenario descriptions.

## Out of scope

- Voxel data structures, edit application, serialization formats and material tables belong in Foundation (World_Core).
- Rendering, GPU memory management, physics integration, destruction mechanics, rigs and ragdolls belong in Engine crates. Gameplay requests these through their public APIs and never reimplements them.
- Process startup, command-line flags, backend choice and plugin assembly belong in `app/`.
- Offline asset conversion belongs in `tools/`.

## Relationships

This folder sits above Foundation and Engine in the dependency order Foundation <- Engine <- Game <- app. Crates here may depend downward on both lower categories. Nothing in Foundation or Engine may depend on anything here. The `app/` binary depends on the crates in this folder and adds the game plugin to the application it builds.

## Rules

- Hot patching is a first-class constraint. The gameplay crate is the most hot-patched crate in the repository (`dx serve --hot-patch`), so its systems run in `Update`, avoid frequent changes to struct layout, and keep tunable values in data assets or the live inspector instead of constants.
- A crate here exposes one Plugin and a prelude, like every Bevy crate in the workspace.
- Few, large crates are preferred over many small ones in this category, because hot patching is slow or fails across many-crate workspaces. Split into a new crate only when a part becomes stable and reusable.
- Gameplay never touches the physics engine's own types directly; it goes through the physics crate's bridge.
- Gameplay writes to the world only through edit operations defined in World_Core, so the CPU tree stays the single source of truth.
- Rasterized triangles are allowed here only for user interface and debug drawing. The world, props and characters stay voxel data.
