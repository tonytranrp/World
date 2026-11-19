# Tests

## Purpose

This workspace-root folder holds cross-crate integration tests and the data they run on. Each layer of the engine is tested against a simpler reference implementation, so the GPU, physics and worldgen code always has something trustworthy to be compared with. Tests that need more than one crate, a real or software GPU, or recorded scenarios live here. Tests for a single crate live in that crate's own `tests/` folder.

## Scope

- `golden/`: reference images for headless render tests.
- `scenes/`: small deterministic scenes used as test inputs: voxel scenes, worldgen seeds and chunks with golden hashes, and physics scenes.
- `replays/`: recorded deterministic replays and the expected per-step transform hashes.
- Integration test code that combines these fixtures with the crates: GPU ray hits compared exactly with the CPU reference renderer on small scenes, headless renders compared with golden images, replay runs checked by hash, worldgen hash checks across platforms.
- Test harness helpers shared by these integration tests.

## Out of scope

- Unit and property tests for one crate (for example the 64-tree against a dense reference array). Those live in that crate's `tests/` or inline.
- Benchmarks and performance-regression gates. Those use Criterion in the crates and the jobs in `.github/workflows`.
- Shipping content. Anything the game loads at runtime lives in `assets/`.
- Test tooling that is a standalone program, which belongs in `tools/Xtask`.

## Relationships

- Depends on `World_Core`, `World_Gen`, `World_Cpu_Ref`, `World_Render`, `World_Physics` and others, as each test requires.
- Run in CI from `.github/workflows` on software Vulkan (lavapipe) and DX12 WARP.
- Fixtures are updated through Xtask commands so regeneration is deliberate and reviewable.

## Rules

- Tests are deterministic: fixed seeds, fixed step counts, no wall-clock dependence.
- Image comparison is tolerance-based; hit comparisons against the CPU reference are exact.
- A golden file or hash changes only with a reviewed explanation of why the output should differ.
- Large binary fixtures are stored through Git LFS.
- Scenes stay small so the suite runs fast on software rendering.
