# Config

## Purpose

This folder holds tool configuration that lives outside Cargo, chiefly the cargo-nextest configuration. It decides how tests are run, as distinct from what the tests contain: which may run in parallel, how long they may take, and whether anything is retried.

## Scope

- Nextest profiles: a default for local runs and a stricter one for CI.
- Test groups: every `gpu_` test shares one group limited to a single thread, so tests that use a GPU or software adapter never run at the same time.
- Timeouts: a threshold that flags slow tests and a hard limit that terminates them, sized for software rendering, which is slower than a real GPU.
- Retry policy: none for deterministic tests. A test that passes only on retry is reported as flaky, not hidden.
- Filter sets that Xtask uses to pick the fast and GPU test sets by name.
- Other tool configuration whose conventional home is this folder.

## Out of scope

- Cargo and workspace configuration, including `.cargo/config.toml`.
- Toolchain pinning.
- CI workflow definitions in `.github/workflows`.
- Test scale, which `WORLD_TEST_PROFILE` and the World_Testkit profile module decide.
- The commands that invoke nextest: Xtask.

## Relationships

Read by nextest, which is invoked by `cargo xtask test`, `cargo xtask check` and CI. Group and filter names rely on the `gpu_` test naming convention used across the workspace.

## Rules

- Configuration changes how the runner schedules tests, never what a test asserts.
- Nextest profile names and `WORLD_TEST_PROFILE` values are separate and are set independently.
- The GPU group stays serialized. Its limit is not raised to speed up a run.
- A timeout is a failure, never a skip.
- Each setting carries a comment that gives its reason.
