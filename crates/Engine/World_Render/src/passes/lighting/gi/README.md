# GI

## Purpose

Global illumination through a world-space radiance cache. Instead of tracing full bounce paths per pixel, irradiance is stored in probes attached to bricks, updated a little each frame, and sampled by every pixel. This gives soft bounce light and emissive spill from lamps and lava without baking, so it stays correct when the world is destroyed.

## Scope

- Probe placement per brick (or per cluster of bricks) and the storage for their irradiance, allocated to follow the brick pool so memory stays fixed.
- Incremental probe updates: a bounded number of probes receive rays each frame, with priority for probes near the camera, probes on recently edited bricks, and probes visible on screen.
- Temporal accumulation of probe results and a denoise to remove noise and boiling.
- Sampling probes for each G-buffer pixel and combining the result with AO.
- Invalidation when voxels change, so edited regions relight over a few frames rather than staying wrong.
- Emissive voxel sources and sky contribution as inputs.
- The tier-A alternative stage (Solari or DLSS-RR) as an optional replacement or supplement, selected by `tier`.

## Out of scope

- Direct sun: `shadow`. Short-range occlusion: `ao`. Specular reflections: `reflections`.
- The brick pool itself: `brick_cache`. GI storage is parallel to it but separate.
- Baked or saved lighting; none exists.

## Relationships

Depends on `brick_cache` residency and edit notifications, `gbuffer`, and shared traversal modules. Feeds the combine stage in `lighting`, then `upscale`. On `--mac-floor` it updates less often.

## Rules

- Probe memory is fixed and tied to pool size, never to world size.
- Update cost per frame is bounded and configurable; stability matters more than convergence speed.
- The result must not boil or flicker at 60 fps on the dev machine or 30 fps on the floor profile.
- Tier B software GI is the baseline; hardware paths never become required.
