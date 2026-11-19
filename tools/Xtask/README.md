# Xtask

## Purpose

Xtask (package `xtask`) is the project's build and automation runner, invoked with `cargo xtask`. It replaces ad hoc scripts with one cross-platform Rust program, so the same commands work on Windows, on macOS and in CI. It is a development tool and never ships with the game.

## Scope

- Codegen: mirroring constants defined in World_Core (voxel size, brick and node dimensions, material limits, palette sizes) into WESL shader modules, so CPU and GPU code can never disagree about layout.
- Bench: running the Criterion benchmarks (tree edits, traversal and others) and comparing results to recorded thresholds, failing when a regression exceeds the allowed margin.
- Package: producing distributables. On macOS, building the x86_64 and aarch64 targets, merging them with `lipo` into a universal binary, bundling the app and assets into a `.app` and `.dmg`, and ad-hoc signing. On Windows, assembling the packaged build.
- Shared plumbing for these commands: argument dispatch, running subprocesses, locating the workspace root, and consistent error reporting.

## Out of scope

- Game and engine code of any kind.
- Asset conversion, which belongs in Bake_Vox and Mesh_To_Voxel (Xtask may call them but does not reimplement them).
- Continuous integration workflow definitions, which call Xtask rather than being part of it.
- Notarization with a Developer ID, which is a release-process step outside this crate.

## Relationships

Depends on World_Core for the constants it mirrors, and on tools it orchestrates. Nothing depends on it. Each command lives in its own subfolder under the source tree.

## Rules

- Commands are idempotent and deterministic. Generated files are byte-stable, so a check mode can detect drift in CI.
- Generated code is never edited by hand and is reproduced from World_Core.
- Platform-specific steps are gated by target operating system and fail with a clear message elsewhere.
- Xtask must not become a dependency of the runtime build.
