# Test Command

## Purpose

This folder implements `cargo xtask test`, the standard way to run the workspace's tests. It chooses a profile, sets the environment variables the testkits read, and runs cargo-nextest, so the same invocation behaves the same on a developer machine and in CI.

## Scope

- Profiles: `fast` (the quick non-GPU tests at the fast random scale), `gpu` (the `gpu_` tests, serialized, on the adapter chosen by `WORLD_TEST_ADAPTER`), `full` (the full random scale, 1,000,000 edits) and `all` (every profile in turn).
- Setting `WORLD_TEST_PROFILE` and `WORLD_TEST_ADAPTER` for the chosen profile, and passing the caller's `WORLD_TEST_REQUIRE_GPU` through unchanged.
- Selecting nextest filters and the nextest profile from the configuration in `.config`.
- Passing extra arguments to nextest, such as a package or a test name filter.
- Reporting a missing cargo-nextest with how to install it, and returning the test run's exit code.

## Out of scope

- Test code, oracles and strategies: the testkit crates.
- Groups, timeouts and retries: the nextest configuration.
- Regenerating goldens: the golden command. Linting and coverage: the check command.
- CI workflow definitions, which call this command.

## Relationships

Called through the Xtask dispatcher and built on its shared subprocess helpers. The variable names it sets are defined by the testkits' profile and policy modules. Plain `cargo test` remains valid for a quick local run, but it applies none of the nextest groups or timeouts.

## Rules

- A profile is the same command locally and in CI. CI adds only the strict-GPU variable.
- The command never changes test code or fixtures.
- It never hides a skip. `SKIPPED` lines stay visible in the output.
- Any failure gives a nonzero exit code.
- It is safe to run repeatedly and from any directory inside the workspace.
