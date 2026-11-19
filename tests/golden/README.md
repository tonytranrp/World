# Golden Images

## Purpose

This folder holds the reference images that headless render tests compare against. A golden image is the accepted output for a named scene, camera and settings. Comparing a fresh render to it catches visual regressions in the voxel renderer that numeric tests would miss.

## Scope

- Reference renders of scenes from `../scenes`, produced by headless runs of the renderer.
- Per-backend variants where acceptable differences exist: software Vulkan (lavapipe) and DX12 WARP produce slightly different pixels, so a comparison uses a tolerance, and a backend that cannot match within it gets its own reference.
- Reference images for individual stages when useful: G-buffer channels such as depth, normal and material id, so a failure points at the pass responsible.
- Metadata that fixes how an image was produced: scene identity, camera, internal and output resolution, tier and settings.

## Out of scope

- Exact GPU-versus-CPU hit comparisons. Those compare data against `World_Cpu_Ref`, not images.
- The scenes themselves, which live in `../scenes`.
- Screenshots for documentation, which belong in `docs/`.
- Throwaway captures from local runs; only reviewed reference images are committed.
- Images the game loads at runtime, which belong in `assets/`.

## Relationships

- Read by integration tests in the parent `tests/` folder; written by an Xtask command that regenerates references.
- Produced by `World_Render` through a headless pipeline, and compared in CI jobs in `.github/workflows` that use lavapipe and WARP.
- Scenes come from `../scenes`; replay output is separate in `../replays`.

## Rules

- A reference changes only with a reviewed explanation of the visual change.
- Comparisons use a documented tolerance; never loosen it to make a failure pass without understanding it.
- Images are stored through Git LFS, in a lossless format.
- Each image's name identifies scene, backend class and stage, so the cause of a diff is clear from the name.
- Keep resolutions small to keep software rendering and storage cheap.
