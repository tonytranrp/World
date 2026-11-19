# World_Game Tests

## Purpose

Integration tests for the gameplay crate. They run the game plugin inside a headless Bevy App and assert on the resulting world and component state, so gameplay logic is verified without a window or a GPU.

## Scope

- Headless ECS tests: build an App with minimal plugins plus the game plugin, step it a fixed number of updates, then assert on state. Examples include a tool use that must remove the expected voxels, a scenario that must spawn the expected bodies, a save followed by a load that must reproduce the same world.
- Domain-level behavior: targeting hits the expected voxel, damage respects material strength, creatures react to events, audio and effect requests are emitted with expected parameters.
- Order independence: tests run with schedule randomization enabled to expose hidden system-ordering assumptions.
- Round trips: edit overlay serialization through `save`, compared against the in-memory world.

## Out of scope

- Voxel core properties belong in World_Core's tests, which compare against a dense reference array.
- Rendering and GPU correctness belong in World_Render's tests, compared against World_Cpu_Ref.
- Physics determinism and replay hashes belong in World_Physics's tests.
- Performance measurement belongs in benches folders.

## Relationships

Depends on the game crate's public surface and the Foundation crates it uses, building scenes through `sandbox`. Does not need the application binary.

## Rules

- References are simple and trustworthy: expected voxel counts, dense-array equivalents, recorded hashes, explicit expected values.
- Tests are deterministic: fixed seeds, fixed step counts, no wall-clock dependence.
- Use only the public API so refactors inside the crate do not break tests.
- No GPU or window requirements; the tests must run on CI without graphics hardware.
