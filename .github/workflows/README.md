# Workflows

## Purpose

This folder contains GitHub Actions workflow definitions and nothing else. It is the project's continuous integration: the automated checks that keep every push buildable, correct and fast on each target platform.

## Scope

- Windows build and tests: building the workspace for `x86_64-pc-windows-msvc` and running unit, property and integration tests.
- Render tests on software Vulkan (lavapipe): headless renders compared with golden images within tolerance, and exact GPU-versus-CPU-reference hit checks on small scenes, plus DX12 WARP where available.
- macOS builds for `x86_64-apple-darwin` and `aarch64-apple-darwin`, the packaging path through `lipo` and bundling, and testing as far as hosted runners allow. The Intel build depends on GitHub continuing to offer Intel runners.
- A performance regression gate: Criterion benchmarks and frame-time checks that fail the build above a set threshold.
- Release and packaging jobs that call `tools/Xtask`.
- Checkout configured to fetch Git LFS content, so images, voxel files and audio are available to tests and packaging.

## Out of scope

- Build, test and packaging logic. Workflows are thin: they set up the toolchain and call `tools/Xtask`, `cargo` or `scripts/`, so the same steps can be run locally.
- A README for the `.github` folder itself, which would override the repository root README on GitHub.
- Secrets. They are stored in repository settings and referenced by name.
- Issue templates and other GitHub configuration, which belong in `.github/` but outside this folder.

## Relationships

- Builds and tests everything: all crates, `tools/`, using fixtures from `tests/` and content from `assets/`.
- Calls `tools/Xtask` and may call `scripts/`.
- Uses `packaging/` metadata for release jobs.

## Rules

- Pin action versions and toolchain versions; upgrade deliberately.
- Cache dependencies and build artifacts to keep runs fast.
- A job that fails locally must fail in CI the same way; avoid CI-only logic.
- Never print or commit secrets.
- Performance thresholds live in versioned configuration, not inline in the workflow.
