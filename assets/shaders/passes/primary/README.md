# Primary Pass Shaders

## Purpose

This folder holds the shaders of the primary-ray pass: the compute stage that decides what is visible. It traces one ray per pixel (or one per 8x8 tile when beam optimization applies) through the sparse 64-tree at internal resolution and writes the result into the G-buffer that every later pass reads.

## Scope

- The compute entry point and its dispatch-tile logic, including any coarse beam pre-pass that finds a safe starting distance for a tile.
- Camera ray generation from the view and projection data, including sub-pixel jitter for temporal accumulation.
- G-buffer output: depth, normal (derived from neighboring voxels at render time, never stored), material id and motion vectors.
- Writing depth into the texture Bevy uses, so rasterized UI, gizmos and particles occlude correctly.
- Detail selection: choosing the mip level at which rays stop, based on distance and pixel footprint.
- Emitting brick requests through the traversal feedback path when rays hit non-resident bricks.

## Out of scope

- Tree walking and DDA. Those are in `../../traversal` and called, not copied.
- Lighting, shadows and GI (`../lighting`), reflections (`../reflections`), upscaling (`../upscale`) and composite (`../composite`).
- Dynamic objects, which are handled by `../dynamic_box` and merged into the same G-buffer.
- Pipeline and bind group setup. That is Rust in `World_Render/src/passes`.

## Relationships

- Imports `../../traversal` and `../../common`.
- Mirrors the primary pass module in `World_Render`.
- Its G-buffer layout is the input contract for lighting, reflections and upscale.
- Output must match the CPU reference tracer in `World_Cpu_Ref` exactly on small scenes.

## Rules

- Hits are exact: no approximation is allowed in the hit position that the CPU reference would not make.
- The pass reads the internal-resolution size from bindings so mac-floor mode works unchanged.
- Runs on compute-only hardware; no optional-feature dependency.
- G-buffer field encodings come from `../../common` and are shared by every consumer.
