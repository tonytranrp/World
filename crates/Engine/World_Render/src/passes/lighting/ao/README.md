# AO

## Purpose

Ambient occlusion: darkening of crevices and contact regions that sit under open sky yet receive little ambient light. At mid range it is the main thing that makes fine voxel surfaces read as solid, and it is cheap enough to run on every tier.

## Scope

- Short hemisphere rays traced from the G-buffer surface through the voxel tree, with a small sample count and a short maximum distance.
- Sampling patterns suited to the pixel grid and temporal accumulation (blue noise or per-frame rotation) so low sample counts resolve cleanly.
- A spatial and temporal denoise of the AO term, using G-buffer normals, depth and motion vectors to avoid bleeding across edges.
- The AO target consumed by the combine stage in `lighting`, and as an input that scales indirect light from `gi`.
- Range and strength parameters, scaled for the voxel size.

## Out of scope

- Direct sun visibility: `shadow`. Indirect radiance: `gi`.
- Screen-space AO approximations. AO here is traced, so it sees off-screen geometry.
- Shared traversal code: WESL modules in `assets/shaders/traversal`.

## Relationships

Depends on `gbuffer` and `brick_cache` for ray access and on `dynamic_box` data for occlusion by moving objects. Runs after primary visibility, alongside `shadow`, and feeds the combine stage in `lighting` and `gi`. Parameters come from `assets/config`.

## Rules

- AO rays reuse the shared traversal; they never use a separate approximate structure.
- Short range keeps the cost low and avoids requesting bricks far from the surface; AO must not generate a flood of cache requests.
- Missing data counts as unoccluded for the frame.
- Sample count and resolution scale down for `--mac-floor` without changing the algorithm.
- Denoising must not blur across material or depth discontinuities.
