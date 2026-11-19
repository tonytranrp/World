# Check Command

## Purpose

This folder implements `cargo xtask check`, the single command that answers whether a change is fit to push. It also holds the entry points for the heavier quality tools, coverage and Miri, which are run on demand rather than on every change.

## Scope

- `check`: a formatting check that does not rewrite files, clippy over the whole workspace and all targets with warnings denied, then the fast test profile. It stops at the first failing stage and names it.
- Coverage: running the tests under cargo-llvm-cov and producing a report.
- Miri: running the tests of crates that contain unsafe code under Miri, with the toolchain Miri requires and without changing the project's pinned toolchain.
- Clear messages when a required tool or toolchain component is not installed.

## Out of scope

- How tests are run: the test command, which check calls.
- Benchmark regression gates: the bench command.
- Updating fixtures: the golden command.
- CI workflow definitions, which call this command.

## Relationships

Called through the Xtask dispatcher and built on its shared subprocess helpers. Uses the fast profile of the test command. Coverage and Miri read the workspace's crates and write reports under the build output folder.

## Rules

- Check never modifies the working tree.
- Warnings are denied outright. No allow attribute is added to get past the check.
- Coverage and Miri are separate opt-in commands and are not part of the default check.
- Miri runs only on crates free of GPU, windowing and foreign-function dependencies.
- Coverage is a review aid and never replaces tests checked against an oracle.
- Any failure gives a nonzero exit code.
