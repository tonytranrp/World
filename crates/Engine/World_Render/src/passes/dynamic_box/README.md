# Dynamic Box

## Purpose

Draws everything that moves as its own voxel grid: debris, props, tools, the player body and creatures. Following the Teardown approach, each object is an oriented box that is rasterized, and its fragment shader marches a ray through the object's dense grid in object space. Results go into the same G-buffer as the static world.

## Scope

- Per-object GPU data: oriented box transform, dense voxel grid storage (a grid per object, possibly finer than the world, such as 1/64 m or finer for hands and held tools), palette and material ids.
- Instance preparation: gathering object boxes extracted from the main world, culling, and uploading instance and grid buffers.
- The rasterization of proxy boxes with depth testing against primary traversal output, and the object-space march in the fragment shader.
- Writing G-buffer outputs (depth, normal, material id, motion vectors) with encodings identical to `primary`, including depth into Bevy's depth texture.
- Producing the box list that `reflections` uses to build its small GPU BVH, so the player appears in reflections.
- Hiding selected parts from the main view only (for example the player's head in first person) while keeping them for shadows and reflections.

## Out of scope

- The static world and its tree: `primary` and `brick_cache`.
- Object physics, mass and islands: World_Physics. Rig animation: World_Anim. This pass only receives transforms and grids.
- Debris smaller than the particle threshold: a particle path, not a grid.
- The reflection BVH itself: `reflections`.

## Relationships

Depends on `gbuffer` and on `primary` having produced world depth. Shadow, AO and reflection passes trace dynamic boxes through it. World_Physics and World_Anim feed it data through extraction in `graph`.

## Rules

- Dynamic data is dense per-object storage; the 64-tree is never used for moving objects.
- Output must be indistinguishable in encoding from world hits.
- Per-object grids are independent of the brick cache and do not count against its pool.
- Per-frame uploads are limited to changed grids and transforms.
