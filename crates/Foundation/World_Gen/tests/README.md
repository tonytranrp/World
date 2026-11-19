# World_Gen Tests

## Purpose

This folder holds the tests that prove generation is deterministic and seamless. Everything the game promises about saves depends on regenerating the same base world every time, so these tests are strict and exact rather than approximate.

## Scope

- Golden hash tests: for a set of seeds and chunk coordinates, the content hash of the generated chunk must equal a recorded value. A mismatch means output changed, which is either a bug or a deliberate world-version change.
- Determinism across execution: the same chunk generated alone, inside a large parallel batch, with different thread counts and in different request orders must be identical.
- Seam tests: voxels at chunk borders match what neighbors expect, including features that cross borders such as caves, rivers and tree canopies.
- Noise primitive tests with exact expected outputs for known inputs, to detect drift in the noise module.
- Property tests: bounded-world edge behavior, level-of-detail consistency for props, and every solid voxel having a valid material.
- Prop tests: the same prop voxelized at different resolutions has matching bounds and similar volume, and SDF primitives match closed-form values.
- Overlay tests: a base chunk with an applied edit overlay reconstructs the saved world, using World_Core.

## Out of scope

- Throughput measurements: the benches folder.
- Rendering and physics checks.
- Tests of World_Core structures themselves.

## Relationships

Tests use only the public API of World_Gen and World_Core. The recorded hashes are the reference: they are produced by the generator on one platform and must reproduce on all supported platforms and CPU architectures.

## Rules

- Seeds and coordinates in tests are fixed and include edge cases such as negative coordinates and world bounds.
- Golden values change only with a deliberate, explained world-version change.
- No test may depend on timing, thread count or platform.
