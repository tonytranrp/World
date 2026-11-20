# Debug

## Purpose

Debugging aids for the renderer: things that make the voxel pipeline visible and testable before the rest of the game feeds it real content, and that help explain what the GPU is doing once it does.

## Scope

- A small built-in test scene, built in a `VoxelTree` and published through the same feed the game uses, shown while nothing else feeds the renderer a world; optionally animated with edits to exercise the edit-upload path.
- Diagnostics of the brick cache and the passes: residency figures, upload and eviction counters, logged at a low rate.
- Later: overlays (residency level, request heat maps, step counts) and GPU timing readouts.

## Out of scope

- The renderer's real data path: `brick_cache` and `passes`. Debug code only observes it or feeds it the way the game would.
- Gameplay test content and saved scenes: `tests/scenes` and the game crates.
- Developer UI panels in general: the developer tools in the game and reload crates.

## Relationships

Depends on `brick_cache` (the feed and statistics) and World_Core (to build the scene). Nothing in release paths depends on it; the plugin wires it behind the `debug_scene` config switch.

## Rules

- Debug features are switched by config or developer builds and never change what the renderer produces for real content.
- The test scene is deterministic: the same frame number gives the same voxels.
- Nothing here holds state the renderer relies on; removing the folder must not break the frame.
