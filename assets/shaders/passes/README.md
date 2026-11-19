# Shader Passes

## Purpose

This folder contains the entry-point shaders of the render pipeline, one subfolder per pass. The layout mirrors `World_Render/src/passes`: for each pass module in Rust there is a folder here with the same name, so someone reading a pass can find its shaders immediately and vice versa.

A frame is built in this order: stream bricks requested last frame, trace primary rays, draw dynamic objects with box-march, light the frame, accumulate and upscale, then composite UI and post effects.

## Scope

- `primary/`: compute pass that traces one ray per pixel into the G-buffer (depth, normal, material id, motion vectors) and writes depth for Bevy's depth texture.
- `dynamic_box/`: oriented-box rasterization whose fragment stage marches the object's dense grid, used for dynamic objects, hands and tools.
- `lighting/`: shadow rays, ambient occlusion and global illumination from the world-space radiance cache.
- `reflections/`: secondary rays through the tree plus a GPU BVH over dynamic-object boxes.
- `upscale/`: temporal accumulation and upscaling from internal to output resolution.
- `composite/`: combining the traced result with Bevy's own passes (atmosphere, UI, gizmos, particles).

## Out of scope

- Shared logic. Tree traversal belongs in `../traversal`; constants, math, packing and material lookup belong in `../common`.
- Pass scheduling, resource allocation and render-graph wiring. Those are Rust in `World_Render`.
- Tunable parameters, which are RON in `assets/config`.

## Relationships

- Each pass imports `../traversal` and `../common`; passes do not import each other.
- Rust counterparts live in `World_Render/src/passes` under the same names.
- Pass outputs are the contract between passes (G-buffer layout, radiance cache layout); changes to them are coordinated in `common` and `World_Render`.

## Rules

- Adding a pass means adding a folder here and a module in `World_Render/src/passes` with the same name.
- Passes contain entry points and pass-specific shading only.
- Every pass runs on tier B (compute-only). Hardware-RT or mesh-shader variants are conditional, never required.
- Resolution-dependent work reads the internal-resolution size from bindings, not constants, so mac-floor mode works unchanged.
