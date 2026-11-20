# World_Cpu_Ref Benchmarks

## Purpose

This folder holds the Criterion benchmarks of the reference tracer. Speed is not a goal of this crate, but test suites and tools trace whole frames through it, so its cost must stay known and reasonable, and it is useful to see how far the hierarchical walk is from the plain dense DDA it is checked against.

## Scope

- Primary rays per second for a fixed camera and scene, over the source types the tracer supports (dense grids and the world tree), at full resolution and at a coarse stop level.
- The same rays through World_Testkit's dense DDA, as the baseline.

## Out of scope

- Correctness: the tests folder.
- GPU timings: World_Render.
- Production ray queries: World_Core's benchmarks.

## Relationships

Benchmarks use the crate's public API, World_Core to build scenes, World_Gen for terrain and World_Testkit for the baseline, all as development dependencies.

## Rules

- Fixed seeds, scenes and cameras, so runs are comparable.
- Scene building happens outside the timed region.
- No optimization lands here that would make the traversal harder to audit; the crate's correctness rule comes first.
