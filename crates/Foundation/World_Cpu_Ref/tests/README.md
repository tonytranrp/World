# World_Cpu_Ref Tests

## Purpose

This folder holds the tests that establish that the reference renderer is itself trustworthy. The GPU is only as correct as this crate, so the reference is checked against analytic geometry and against the simplest possible traversal before anyone relies on it.

## Scope

- Analytic scene tests: spheres, planes, boxes and cylinders voxelized through World_Core, with ray hit distances and normals compared against closed-form intersection results within the error voxelization allows.
- Naive-path equivalence tests: the hierarchical reference traversal must match the check-every-voxel path exactly on random small trees and random rays, including rays that graze faces, edges and corners.
- Level-limited traversal tests: coarse rays must agree with mip data from World_Core.
- Camera tests: ray generation for known pixels, pixel-center conventions, previous-frame motion vectors, and rejection of degenerate cameras.
- Output tests: PNG encoding is deterministic, comparison reports mismatches correctly, and hashing is stable.
- Rendering of the named test scenes to PNG, kept as golden references for human review and for comparison with GPU captures.

## Out of scope

- GPU versus reference comparisons: those tests live with World_Render because they need a device.
- Tests of tree correctness: World_Core tests.
- Performance measurements: this crate does not optimize for speed.

## Relationships

Tests use World_Cpu_Ref's public API, World_Core to build scenes and, where useful, World_Gen as a development dependency. Golden data produced here is the input that World_Render's comparison tests consume.

## Rules

- Scenes are small so tests run on every push and mismatches are easy to inspect.
- Expectations come from analytic math or from the naive path, never from the code under test.
- Golden outputs are deterministic, and changing one requires an explanation in the change that updates it.
- Every GPU discrepancy found later is reduced to a scene here and kept as a permanent regression test.
