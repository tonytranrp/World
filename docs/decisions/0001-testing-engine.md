# 0001: Testing engine

Status: accepted, 2026-11-18

## Context

The plan tests every layer against a simpler reference: the 64-tree against a
dense array, the GPU tracer against a CPU reference, physics and worldgen by
recorded hashes, rendering by golden images. None of the code under test exists
yet, so the testing engine is built first, together with the reference models
it needs, so that each subsystem is born with its checks.

## Decision

- **Two support crates.** `World_Testkit` (Foundation, no Bevy) holds the dense
  reference model, proptest strategies, xxh3 hashing, replay records, golden
  comparison and the CPU-vs-GPU hit comparator. `World_Testkit_Bevy` (Engine)
  holds the headless `App` harness, schedule randomization and offscreen
  rendering. Core crates can use the first as a dev-dependency without gaining a
  Bevy dependency.
- **Tests live in three places.** Inline `#[cfg(test)]` units, each crate's
  `tests/` for integration and property tests, and the root `tests/` crate
  (`world_tests`) for cross-crate suites and their fixtures.
- **proptest** for property tests, because it shrinks failing edit sequences.
- **cargo-nextest** as the standard runner (process per test, named profiles,
  test groups). Plain `cargo test` keeps working.
- **Own golden helper.** Tolerance compare, diff image on failure, rewritten
  only with `UPDATE_GOLDEN=1`, which `cargo xtask golden --update` sets. A
  missing golden file fails; it is never created silently.
- **Software adapter by default** for render tests (WARP on Windows, lavapipe on
  Linux), so images match on every machine. `WORLD_TEST_ADAPTER=gpu` uses the
  real GPU with a looser tolerance.
- **GPU tests are named `gpu_*`.** A missing adapter prints `SKIPPED` locally and
  fails when `WORLD_TEST_REQUIRE_GPU=1`, which CI and `golden --update` set.
- **xxh3-64** for state hashes: its output is frozen across versions and
  platforms, so recorded hashes survive toolchain upgrades.
- **Two scales.** `WORLD_TEST_PROFILE=fast` (256 cases of 32 edits) for every
  commit, `full` (1,000 cases of 1,000 edits, the plan's one million random
  edits) before a release.
- **criterion** benchmarks with a regression gate in Xtask that compares against
  a saved baseline.
- **Xtask** is the entry point: `test`, `golden`, `bench`, `check`, `coverage`,
  `miri`.
- **Schedule randomization** uses Bevy's `shuffle_seed`, available behind Bevy's
  `debug` feature, which only `World_Testkit_Bevy` enables.

## Consequences

- The oracle must itself be tested; `World_Testkit/tests` checks the dense model
  against a hash-map model and its ray traversal against a per-voxel slab test.
- Swapping the schedule executor loses the final apply of deferred commands.
  The harness restores it, and a regression test guards it.
- Golden images are only stable on the software adapter, so a Mac without one
  runs render tests with `WORLD_TEST_ADAPTER=gpu`, or skips them locally.
- Enabling Bevy's `debug` feature in the workspace test build changes the
  feature set, so the first workspace-wide test build recompiles Bevy once.
