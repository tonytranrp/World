# Reflections

## Purpose

Secondary rays for mirror-like and glossy surfaces such as water and polished floors. Reflection rays reuse the primary traversal against the voxel pool, and additionally test dynamic-object boxes through a small GPU BVH so that the player body and props appear in reflections.

## Scope

- Selection of reflective pixels from material data (roughness and smoothness in the material table, read through the material id).
- Reflection ray generation from G-buffer normals and view direction, with roughness-driven spreading so sharpness ranges from mirror to painterly blur.
- Tracing the reflected ray through the voxel world with the shared traversal, using coarse mips where distance allows.
- Building and updating the small GPU BVH over dynamic-object oriented boxes each frame, and testing reflection rays against it before marching into an object's grid.
- Shading the hit (world or dynamic object) with simplified lighting and fog, and compositing it with the base lit image by Fresnel and roughness.
- Reduced mode: half resolution and smooth materials only on `--mac-floor`, with a temporal filter to hide the lower resolution.

## Out of scope

- Primary visibility and lighting of the base image: `primary`, `dynamic_box`, `lighting`.
- The grids and instance data of dynamic objects: `dynamic_box` owns them and this folder consumes the box list.
- Screen-space reflections, planar cameras or cubemap probes.

## Relationships

Depends on `gbuffer`, `brick_cache` pool access and `dynamic_box` data. Runs after lighting and before `upscale`. Shaders mirror this folder under `assets/shaders/passes/reflections`.

## Rules

- The player body must always be representable in reflections, including parts hidden in the first-person view.
- Reflection rays use the same traversal code as primary rays; no separate approximation of the world.
- Cost is bounded by resolution and material filter, never by scene complexity.
- Missing pool data produces a sky or fog fallback and a feedback request, not a stall.
