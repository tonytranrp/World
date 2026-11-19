# Sandbox

## Purpose

The sandbox domain provides the ways to set up and play with the world as a destruction playground: scenarios, test arenas and creative-mode controls. It exists so that destruction, physics and rendering can be exercised on demand, by a player or by a test.

## Scope

- Scenario descriptions as data: a tower, a wall, a bridge, a pile of crates, a building to demolish, with parameters such as size, material and placement.
- Spawning: building dynamic voxel objects and static structures into the world through World_Core edits and World_Physics body creation.
- Test arenas: small bounded scenes with known contents, used for manual play and for headless tests, optionally using World_Gen for terrain.
- Creative-mode controls: spawn and delete objects, toggle time of day, reset a scene, pause or slow physics, and spawn at a chosen material.
- Scenario reset and repeat, so the same destruction can be run again.

## Out of scope

- Terrain and prop generation algorithms belong in World_Gen.
- Menus and panels that expose these controls belong in `ui`.
- Tool behavior belongs in `tools`.
- Persistence of player edits belongs in `save`.

## Relationships

Depends on World_Core, World_Gen and World_Physics. Uses `interaction` to apply edits so spawning follows the same path as player actions. Read by `ui` for control lists. Used by the tests folder to build deterministic scenes.

## Rules

- Scenarios are deterministic: the same scenario and seed produce the same voxels.
- Spawned content enters the world only through the normal edit and body creation paths.
- Scenarios are data driven and hot-reloadable; adding one should rarely require code.
- Creative-only controls are gated, so they cannot change world state in normal play.
- Keep systems in `Update` and avoid changing live component layouts.
