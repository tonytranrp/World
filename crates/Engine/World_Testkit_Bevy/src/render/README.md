# Render

## Purpose

This module gives tests a GPU to render on and a way to get the pixels back. It chooses the adapter, sets up an offscreen target and captures the result as raw RGBA frames for golden comparison. It defaults to a software adapter, so rendered images are stable from one machine to the next.

## Scope

- Adapter selection from `WORLD_TEST_ADAPTER`: `software` (the default) picks WARP on Windows and lavapipe on Linux, and `gpu` picks the machine's real adapter.
- Applying the choice to Bevy's render setup before the device is created.
- Reporting a missing adapter to World_Testkit's skip policy, so the test skips loudly, or fails when `WORLD_TEST_REQUIRE_GPU=1`.
- An offscreen camera target of a fixed, small resolution, with no window or swapchain.
- Screenshot capture after a requested number of frames, returned as tightly packed 8-bit RGBA with GPU row padding removed.
- Recording which adapter class produced a frame, so golden names can carry software or gpu.

## Out of scope

- Comparing frames, tolerances and diff images: World_Testkit golden.
- The renderer under test: World_Render and its shaders.
- Scenes and cameras for particular tests: the tests themselves.
- Screenshots of the playable game.

## Relationships

Builds on app for stepping frames and on World_Testkit for the skip policy. Used by render tests in World_Render and by the render-golden and GPU suites in the root `tests/` folder.

## Rules

- Software by default. Using the real GPU is an explicit choice.
- Tests that need an adapter carry the `gpu_` name prefix.
- Resolution and frame count are explicit arguments. The same inputs on the same adapter class give the same pixels.
- The adapter is fixed before device creation and never switched afterwards.
