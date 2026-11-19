# World_Testkit_Bevy

## Purpose

World_Testkit_Bevy is the Bevy-side counterpart of World_Testkit. It supplies what tests need to run Bevy code without a window: a headless App harness, schedule-order randomization with seeds, adapter selection for offscreen rendering, and screenshot capture into raw frames that golden comparison can read. It produces the situations and the pixels. Judging them is left to World_Testkit. It is a development dependency only and never ships with the game.

## Scope

- A headless App builder on MinimalPlugins that steps a chosen number of frames, runs until a condition on the world holds, and exposes the world for assertions.
- Turning on system-order randomization with a seed, and running one test across several seeds, to flush out hidden ordering assumptions.
- Test adapter selection: a software adapter by default so images are stable (WARP on Windows, lavapipe on Linux), and the machine's real GPU on request through `WORLD_TEST_ADAPTER=software|gpu`.
- Offscreen render targets and screenshot capture, producing raw RGBA frames.
- Wiring to World_Testkit's skip policy, so a missing adapter skips loudly, or fails when `WORLD_TEST_REQUIRE_GPU=1`.

## Out of scope

- Comparing frames with golden images, hashing, test profiles and the skip decision itself: World_Testkit.
- The reference voxel model, strategies and replay record format: World_Testkit.
- The behavior under test. Plugins, systems and renderers belong to the crates being tested.
- Fixture files and cross-crate suites: the root `tests/` folder.
- Runner configuration such as test groups and timeouts: the nextest configuration and Xtask.

## Relationships

An Engine-layer crate. It depends on Bevy and on World_Testkit, and on no other engine crate, so any crate can use it as a dev-dependency without a cycle. It is used as a dev-dependency by the Engine crates, World_Game, the app and the root `tests/` crate. Nothing that ships depends on it.

## Rules

- Dev-dependency only, never imported by shipping code.
- The adapter is chosen before Bevy creates the GPU device, the same order the app itself uses at startup.
- Runs are deterministic: a fixed time step, fixed resolutions and fixed seeds. The wall clock never decides a result; it only bounds how long a test waits for background work such as asset loading or file watching (decision 0002).
- Anything that waits for a condition has a frame or time cap and fails with a description of the world instead of hanging.
- A small prelude is the public surface. Where the harness must register something in an App, it does so through a single test-only plugin.
