# World_Gen Source

## Purpose

This folder is the module tree of the generation crate. It defines the crate's public surface and arranges the code into folders that follow the generation stages and the shared machinery under them. A reader should be able to find the code for any visible feature of the terrain by following the stage it belongs to.

## Scope

- The crate entry point and its small public API: build a generator from a seed and parameters, ask it for a chunk, and get voxels back in World_Core form.
- The parameter types describing generation settings, which higher layers fill from RON assets.
- A world-generation version identifier that changes when authoritative output changes on purpose.
- One subfolder per concern: noise (deterministic primitives), continental (macro terrain), regional (meso features), props (SDF-based objects), surface (material layering), and chunk (orchestration and addressing).

## Out of scope

- Voxel storage, edits and serialization: World_Core.
- Asset loading, file watching and live parameter editing: Engine crates.
- Rendering, physics and gameplay.
- Test and benchmark code: the sibling tests and benches folders.

## Relationships

Stages form a pipeline that the chunk module drives: continental feeds regional, props are placed using both, and surface assigns materials last. Every stage uses noise and nothing else uses the stages upward. Stage modules do not depend on one another sideways except through the explicit data they hand forward.

## Rules

- All code in authoritative paths obeys the determinism rules of the crate: same seed, parameters and chunk coordinate give bit-identical voxels.
- Stages consume deterministic random streams derived from seed, stage name and coordinates, never shared mutable generators.
- Parameters are read-only data passed in; the crate performs no file or network access.
- Each stage is a pure function of its inputs so it can be tested, hashed and parallelized independently.
- Float behavior that could differ across machines is avoided or implemented explicitly in the noise module.
