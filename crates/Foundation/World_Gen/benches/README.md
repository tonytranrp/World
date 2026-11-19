# World_Gen Benchmarks

## Purpose

This folder holds the Criterion benchmarks for world generation. Chunks must be produced fast enough that the player can walk across the world without waiting, and on the performance-floor Mac with fewer worker threads. These benchmarks measure that cost and catch regressions before they reach players.

## Scope

- End-to-end chunk generation time, for typical terrain, cave-heavy chunks, water chunks and prop-dense forests, reported per chunk and per voxel.
- Per-stage cost: continental tiles, regional features, props, voxelization and surface assignment, so a slowdown can be pinned to a stage.
- Noise primitive throughput, including fractal Brownian motion and domain warping at representative octave counts.
- Prop cost: SDF evaluation, grammar expansion and voxelization at several resolutions.
- Parallel scaling: the same batch of chunks with one, four and many worker threads, to confirm scaling and to model the capped thread count of the Mac floor.
- Memory per chunk during generation.

## Out of scope

- Determinism and seam checks: the tests folder.
- Tree edit and traversal timings: World_Core benchmarks.
- GPU timings and streaming: World_Render.

## Relationships

Benchmarks use only the public API plus development dependencies. They are consumed by the CI performance gate and by developers tuning parameters.

## Rules

- Fixed seeds and coordinates make runs comparable.
- Setup, such as building parameter structs and warming caches, is excluded from timed regions, but cold-cache cost is measured separately.
- Benchmarks must not weaken determinism to gain speed; any optimization has to pass the golden hash tests first.
- Results are judged against the budget for a low-end machine, not only the development PC.
