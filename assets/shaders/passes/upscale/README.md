# Upscale Pass Shaders

## Purpose

This folder holds the shaders that take the lit image at internal resolution and produce the output-resolution image. Ray tracing cost depends on pixel count, so the game renders internally at a lower resolution than the display, such as about 720p on the performance-floor laptop with a 3072x1920 panel, and reconstructs detail temporally.

## Scope

- Temporal accumulation: reprojecting the previous output using motion vectors and blending in the new jittered sample.
- History validation and rejection: depth, normal and material-id comparisons to avoid ghosting after disocclusion or destruction edits.
- Reconstruction and sharpening to the output resolution.
- Handling of internal-to-output ratio as a runtime parameter, so one shader serves every tier.
- Optional vendor upscaler integration points (such as DLSS on supported GPUs) as conditional alternatives to the built-in path.

## Out of scope

- Tracing, lighting and reflections, which happen at internal resolution in other passes.
- UI and post effects drawn at output resolution, in `../composite`.
- History buffer allocation and resizing. That is Rust in `World_Render`.
- Choosing internal resolution; that comes from tier and mac-floor settings in `assets/config`.

## Relationships

- Reads the lit frame plus G-buffer motion vectors and depth from earlier passes.
- Mirrors the upscale pass module in `World_Render/src/passes`.
- Imports `../../common` for packing and math.
- Output feeds `../composite`.

## Rules

- Resolution ratios are read from bindings, never compiled in.
- The built-in path must work on compute-only baseline hardware; vendor paths are optional and behind conditionals.
- History is invalidated explicitly on camera cuts and on large world edits, never silently reused.
- No tuning constants in code; blend factors and sharpness come from config.
