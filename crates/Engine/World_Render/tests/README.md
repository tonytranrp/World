# World_Render Tests

## Purpose

Integration tests for the World_Render crate. They exercise the renderer through its public plugin and prelude, the way World_Game and `app` use it, and check the output against references that are simpler and more trustworthy than the GPU code.

## Scope

- Exactness tests: on small scenes, GPU primary-ray hits (depth, normal, material id) must match the World_Cpu_Ref reference renderer exactly, since the CPU 64-tree and its brute-force traversal are ground truth.
- Golden-image tests: headless renders of fixed scenes compared against stored images within a tolerance, run on software Vulkan (lavapipe) and DX12 WARP in CI.
- Brick cache behavior: feedback requests, streaming under the MB-per-frame budget, LRU eviction and fixed pool bounds, driven with scripted camera paths.
- Tier tests: capability detection against fake adapter feature and limit sets, including the `--mac-floor` profile and the tier C error path.
- Headless Bevy App tests with MinimalPlugins that step frames and assert on render-world resources, with schedule randomization enabled to expose ordering assumptions.
- Round-trip tests that the generated shader constants agree with World_Core's constants.

## Out of scope

- Unit tests of private functions: they sit in `#[cfg(test)]` modules next to the code.
- Tree correctness and property tests: World_Core. Reference renderer correctness: World_Cpu_Ref.
- Performance measurement: `benches`, and GPU timing through the debug module.

## Relationships

Depends on world_render, World_Core and World_Cpu_Ref as dev-dependencies. Nothing depends on it.

## Rules

- Every GPU result is compared to a reference; a test with no reference is a smoke test and must say so.
- Scenes are small and generated from seeds or fixed edit lists so failures reproduce.
- Golden images change only by deliberate regeneration reviewed with the diff.
- Tests must run without a discrete GPU or hardware ray tracing.
