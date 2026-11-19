# Suites

## Purpose

This folder holds the cross-crate integration tests of the workspace. Each file is its own test binary and so its own suite, visible and selectable by name in nextest. Names are the contract between a suite and the runner.

## Scope

- Render golden: headless renders of fixture scenes compared with golden images within tolerance.
- Replay determinism: scenarios run twice and checked against recorded replays, including under schedule randomization.
- Plugin behavior: headless App tests of how plugins from several crates work together.
- CPU-versus-GPU hit comparison: exact equality of ray hits on small scenes.

## Out of scope

- Tests of one crate: that crate's own tests folder or inline tests.
- Fixture files: the `golden`, `scenes` and `replays` folders.
- Shared helpers: the sibling source folder.
- Benchmarks and performance gates.

## Relationships

Suites use the helpers in the sibling source folder, both testkit crates, and the engine crates under test. Nextest selects them by name, with `gpu_` as the filter for GPU suites. CI runs the GPU suites on software adapters with `WORLD_TEST_REQUIRE_GPU=1`.

## Rules

- One concern per file, named for what it checks.
- A suite that needs a GPU adapter, software or real, carries the `gpu_` prefix, and GPU suites run serialized.
- Deterministic: fixed seeds and fixed step counts.
- Image checks are tolerance-based. Hit and hash checks are exact.
- A suite never writes fixtures except through the update workflow.
