# World_Physics tests

## Purpose

This folder holds the integration tests for the physics crate. They treat the crate through its public Plugin and prelude and check that the whole pipeline (colliders, stepping, destruction, character) behaves correctly. Unit tests of a single function live beside the code in the source tree, not here.

## Scope

- Deterministic replay tests: a scenario is run twice from the same initial state and the same inputs, the transforms of all bodies are hashed after every step, and the two hash sequences must match exactly.
- Regression scenes with recorded hashes: for example a tower collapse, a wall breaking into pieces, and a pile settling. The recorded hash sequences are checked in as expected data, and a change in behavior shows up as a hash mismatch.
- Island detection tests, compared against a brute-force flood fill over a dense reference array, including randomized edit sequences.
- Collider tests: chunk colliders match the collision grid derived from the tree, and rebuilds equal fresh builds.
- Mass property tests: mass, center of mass, and inertia of voxel bodies against closed-form values for simple shapes.
- Character controller scenes: stepping up 0.3 m ledges, stairs, slopes, and rough voxel ground without catching or jitter.
- Budget tests: time-sliced work completes over several frames without exceeding the per-frame voxel limit, and the active-body cap holds.
- Headless App tests using `MinimalPlugins`, with schedule randomization enabled so hidden ordering assumptions surface.

## Out of scope

- Measuring speed: benches.
- Render output and golden images: World_Render.
- Voxel-core property tests: World_Core.

## Relationships

Depends on the crate's public API, World_Core, and World_Cpu_Ref where a reference is needed. Nothing depends on this folder.

## Rules

- Tests are headless and need no GPU or window.
- Hashes cover quantized body state in a fixed order; expected hashes are updated only with a deliberate, reviewed change in behavior.
- Randomness uses fixed seeds.
- A test compares against a simpler reference, never against its own output computed the same way.
