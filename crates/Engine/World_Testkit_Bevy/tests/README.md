# World_Testkit_Bevy Tests

## Purpose

This folder tests the harness itself. The stepping, ordering and capture tools must be right before any other crate relies on them, so they are checked here through their public API.

## Scope

- App tests: exactly N updates run, run-until stops once its condition holds and fails at the cap with a description of the world, and fixed-step time advances identically on every run.
- Schedule tests: a deliberately order-dependent pair of systems passes with explicit ordering and fails under some seed, the failing seed is reported, the same seed reproduces the failure, and the same seed gives the same order.
- Render tests: the software adapter is selected by default, the real GPU on request, and a flat-color scene is captured as exactly the expected pixels, with the frame's size, packing and channel order checked. These carry the `gpu_` prefix.
- Policy wiring: a missing adapter skips loudly in lenient mode and fails in strict mode.

## Out of scope

- Tests of engine crates that use the harness: their own tests folders.
- Golden comparison logic: World_Testkit tests.
- Cross-crate suites: the root `tests/` folder.

## Relationships

Tests use only the public API of the crate, Bevy and World_Testkit. GPU-dependent tests run through the serialized GPU group of the nextest configuration.

## Rules

- Tests are small and deterministic.
- GPU-dependent tests start with `gpu_` and honor the skip policy.
- Pixel checks use scenes that have exactly one correct image, such as flat colors, so no golden files are needed to test the harness.
- Every test is reproducible: fixed or printed seeds, fixed frame counts.
