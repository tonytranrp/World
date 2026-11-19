# Primary

## Purpose

The primary-ray pass: for each pixel (or each 8x8 tile when beam optimization is used) a compute shader traces a ray through the sparse 64-tree mirrored in the brick pool and records what is visible. This is the foundation of the image; every later pass reads its output.

## Scope

- Dispatch and pipeline setup for the compute traversal at the internal resolution, including tile and beam variants.
- Camera ray generation, including jitter for temporal accumulation and the first-person camera that sits inside the world.
- Writes into the G-buffer: depth, normal (from neighboring voxels at render time), material id, and motion vectors from previous-frame camera data.
- Writing depth into Bevy's depth texture so rasterized UI, gizmos and particles occlude correctly.
- Level-of-detail selection: stopping rays at coarse mips with distance, and emitting missing-brick requests to the feedback buffer on a pool miss.
- Per-pass debug counters (steps, nodes visited) published to `debug`.

## Out of scope

- The traversal algorithm source: shared WESL modules in `assets/shaders/traversal`, imported here and by other tracing passes.
- Dynamic objects and the player body: `dynamic_box`.
- Lighting, reflections, accumulation: other pass folders.
- Pool management and streaming: `brick_cache`.

## Relationships

Depends on `brick_cache` (pool and feedback) and `gbuffer` (targets), and is scheduled first among the passes by `graph`. `dynamic_box`, `lighting`, `reflections` and `upscale` all consume its output. It must agree exactly with World_Cpu_Ref on test scenes.

## Rules

- GPU hits must match the CPU reference renderer exactly; the shared traversal code is the single implementation.
- The pass never blocks on missing data: it shades with the coarsest available level and requests the rest.
- Output encodings are those defined by `gbuffer`.
- Tier B compute is the only requirement; no tier-A feature is used here.
