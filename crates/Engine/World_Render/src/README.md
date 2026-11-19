# World_Render Source Root

## Purpose

This folder is the module tree of the World_Render crate. It holds the crate's public surface (the one `Plugin` and the prelude) and one subfolder per rendering concern. The layout is deliberately few crates with deep module folders: each concern is a folder that can be patched or replaced independently, which suits hot-patching of systems during development.

## Scope

- The crate-level wiring: the plugin entry point that registers tier detection, the brick cache, the render-graph nodes and the pass plugins in the right order, and the prelude re-exporting what the rest of the game may use.
- Module folders, each owning one concern: capability tiers (`tier`), the ray-guided brick cache (`brick_cache`), the G-buffer (`gbuffer`), the render graph integration (`graph`), the render passes (`passes`), and debugging aids (`debug`).
- Small crate-wide items that belong to no single concern: shared error types, the settings resource consumed by all modules, and re-exports of World_Core types the renderer needs.

## Out of scope

- Shader source: it lives in `assets/shaders`, mirroring `passes/` by name, and is loaded through Bevy's asset system with hot reload.
- Tests and benchmarks: sibling folders `tests` and `benches` at the crate root.
- Logic that belongs to the CPU tree, edits or materials: World_Core.
- Anything about physics, animation or gameplay.

## Relationships

Modules depend downward and sideways only through narrow interfaces. `passes` consume `gbuffer`, `brick_cache` and `tier`. `graph` schedules the nodes that `passes` provide. `debug` observes everything and is depended on by nothing in release paths. `tier` depends on nothing else in the crate and is read by everyone. Externally this folder depends on World_Core and Bevy and is consumed by World_Game and `app` through the prelude only.

## Rules

- Exactly one public Plugin and one prelude; everything else is `pub(crate)` or narrower.
- Module folders are snake_case, and each pass folder name matches its shader folder under `assets/shaders/passes`.
- No module reaches into another module's internals. Cross-module data moves through resources, render-world bind groups or explicit types owned by the producing module.
- Rapier, physics types and gameplay types never appear here.
- Keep systems small and free of long-lived state in function bodies so hot-patching stays viable; persistent state lives in resources.
- A new rendering concern gets a new folder with its own README rather than growing a catch-all module.
