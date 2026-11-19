# Dynamic Box Pass Shaders

## Purpose

This folder holds the shaders that draw dynamic objects: debris, props that were freed, the player's body parts, hands and held tools. Dynamic objects are not part of the world tree. Each is a small dense voxel grid with a transform, drawn the way Teardown draws its objects: rasterize the object's oriented bounding box, then march a ray through the object's own grid in the fragment stage.

## Scope

- The vertex stage that places a unit box with the object's transform, including back-face handling so rays starting inside the box (an object filling the screen) still work.
- The fragment stage that converts the pixel ray into object space and calls box-march from `../../traversal`.
- Writing hit results into the same G-buffer layout as the primary pass, with depth merged against the world so objects and terrain occlude each other correctly.
- Motion vectors for moving objects, using the previous-frame transform.
- Per-object data access: grid location in the dynamic-grid buffer, voxel scale (possibly finer than the world, such as 1/128 m), and palette.

## Out of scope

- The world tree and its traversal. That is `../primary` plus `../../traversal`.
- Lighting and reflections. Those read the G-buffer afterward. The BVH over dynamic-object boxes used by reflection rays belongs to `../reflections`.
- Allocating dynamic grids, uploading them and building instance lists. Those are Rust in `World_Render`, fed by `World_Physics` and `World_Anim` transforms.

## Relationships

- Imports `../../traversal` (box-march) and `../../common`.
- Mirrors the dynamic_box pass module in `World_Render/src/passes`.
- Output is merged with `../primary` output before lighting.

## Rules

- This is the only pass in which triangles (proxy boxes) are rasterized for scene content, and they exist only to start rays.
- Hit exactness matches the CPU reference for the same grid and transform.
- Per-object voxel scale is data, never a constant.
- Must work on compute-and-raster baseline hardware with no optional features.
