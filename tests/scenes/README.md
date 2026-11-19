# Test Scenes

## Purpose

This folder holds small, deterministic scenes that integration tests load as input. A scene is a complete, reproducible starting state: given the same scene, every run on every machine yields the same result, which is what lets tests compare the GPU with the CPU reference, physics with its recorded hashes and worldgen with its golden hashes.

## Scope

- Voxel scenes: small hand-built or generated voxel volumes used to check traversal and rendering, covering the cases that break tracers (thin walls, grazing rays, tree node boundaries, mip transitions, empty space, fully dense bricks).
- Worldgen scenes: seeds and chunk coordinates together with golden hashes of the generated voxels, to catch nondeterminism between runs and between Windows and macOS.
- Physics scenes: starting configurations such as the 64 x 64 m terrain with 200 debris pieces used for validating the physics engine, and the tower collapse used as a regression scene.
- Dynamic-object scenes with grids and transforms for the box-march and reflection tests.
- Camera and settings definitions that go with each scene.

## Out of scope

- Reference images. Those are in `../golden`.
- Recorded replays and their expected per-step hashes. Those are in `../replays`; a replay refers to a scene here.
- Game content: models, config and levels that ship. Those are in `assets/`.
- Large scenes meant for stress or performance measurement; keep correctness scenes small, and keep benchmark inputs with the benchmarks.

## Relationships

- Loaded by integration tests in the parent `tests/` folder, using `World_Core`, `World_Gen`, `World_Physics` and `World_Render`.
- Referenced by `../golden` and `../replays`.
- Regenerated through `tools/Xtask` when the intended format changes.

## Rules

- Scenes are deterministic: explicit seeds, explicit parameters, no reliance on wall-clock time or machine state.
- Each scene targets a specific behavior and says which in its metadata.
- Keep them small enough to run quickly on software rendering.
- Binary scene data goes through Git LFS; text descriptions remain plain text.
- Changing a scene invalidates its goldens and replays, so change them together.
