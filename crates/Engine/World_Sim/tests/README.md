# World_Sim tests

## Purpose

This folder holds the integration tests for the simulation crate. They exercise the crate through its Plugin and prelude and verify each simulation against a simpler, trustworthy reference. Per-function unit tests live beside the code in the source tree.

## Scope

- Cellular automaton tests for sand: the same initial grid is run through the crate and through a dense-array reference automaton with identical rules and seed, and every step must match, including pile shape and conservation of voxel count.
- Fire tests: ignition, spread, and burn-out on small grids with controlled random seeds, checked for deterministic outcomes, correct products, and extinguishing.
- Stress tests: small structures such as beams, arches, and overhangs, whose support loads are computed by brute force and compared to the graph solver, and the expected failure when a support is removed.
- Fluid tests: volume conservation, settling to a level surface in a container, and active-cell caps.
- Budget tests: total work per frame never exceeds the allowance, allocation shares are respected, and starved simulations eventually progress.
- Interaction tests: sand burying a structure, fire weakening a support until it fails, and edits flowing into World_Physics so colliders and islands update.
- Headless App tests using `MinimalPlugins` with schedule randomization enabled, to confirm results do not depend on system order within a frame.

## Out of scope

- Rigid-body physics behavior and regression hashes: World_Physics tests.
- Rendering output: World_Render.
- Voxel tree property tests: World_Core.

## Relationships

Depends on the crate's public API, World_Core, World_Physics, and World_Cpu_Ref where a reference is useful. Nothing depends on this folder.

## Rules

- Tests are headless and use fixed seeds.
- Every simulation test compares to an independent reference or a closed-form expectation.
- Scenes are small enough to inspect by hand.
- Conservation checks (voxels, volume) accompany every behavior test that moves material.
