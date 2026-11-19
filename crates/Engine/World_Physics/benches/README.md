# World_Physics benches

## Purpose

This folder holds Criterion benchmarks for the physics crate. The physics step must fit a 60 Hz budget on a low-end machine, and the voxel-proportional work around it must stay inside its per-frame time slice. Benchmarks give those claims a number and let CI detect regressions.

## Scope

- Physics step cost for the reference load: a 64 x 64 m voxel terrain collider, about 200 debris pieces of 2,000 to 20,000 voxels each, and one character controller, run at a capped thread count approximating a base laptop.
- Chunk collider build and rebuild cost as a function of chunk occupancy and edit size.
- Dynamic collider construction and mass-property summation across object sizes.
- Island detection cost: union-find and flood fill across object sizes and edit patterns, including worst cases such as a thin bridge being cut.
- Anchor search cost on large connected terrain.
- Damage application cost for large volumes.
- Debris conversion and particle emission throughput.
- Character controller update cost on rough ground.
- Time-slice behavior: cost per N voxels, to confirm the budget setting holds a frame time.

## Out of scope

- Correctness: tests.
- Rendering or GPU timing: World_Render.
- Core tree edit and traversal benchmarks: World_Core.

## Relationships

Depends on the crate's public API and World_Core. Results feed the CI regression check that fails a build when frame time regresses above a threshold. Nothing depends on this folder.

## Rules

- Benchmarks use fixed seeds and fixed scenes so runs compare.
- Each benchmark states the thread count it uses and caps it for the reference case.
- Setup cost is excluded from the measured section.
- A benchmark measures one concern, so a regression points at one module.
