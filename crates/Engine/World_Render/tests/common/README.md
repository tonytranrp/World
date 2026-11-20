# World_Render Test Support

## Purpose

Shared helpers for World_Render's integration tests and its GPU timing benchmark, so every suite builds pools, devices and pipelines the same way the plugin does.

## Scope

- CPU setup: a small pool layout, a residency table with CPU pools, making a source fully resident, applying batches to the CPU model of the apply pass.
- A bare wgpu device on the adapter `WORLD_TEST_ADAPTER` selects (software by default), with `skip_or_fail` when none exists.
- Compiling the real WESL shaders from `assets/shaders` with the options Bevy uses, and building the trace, apply and eviction pipelines from World_Render's own bind group layouts.
- A GPU cache that uploads batches through the staging belt and the apply pass, blocking read-back of buffers, and tracing rays in test mode.

## Out of scope

- Assertions and scenes: they belong to the suites that use them.
- Bevy apps: offscreen rendering comes from World_Testkit_Bevy.
- Anything shipped: this folder is compiled only into tests and benches.

## Relationships

Depends on World_Render's `internals`, World_Testkit (policy) and World_Testkit_Bevy (adapter choice), and on the `wesl` compiler as a dev-dependency. Used by every file in `../` and by `../../benches/gpu_timing.rs`.

## Rules

- Helpers may block on the GPU; the renderer never does.
- Pipelines and layouts come from World_Render's own code and shaders, never from copies, so tests exercise what ships.
- No test logic or expectations here.
