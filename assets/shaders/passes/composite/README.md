# Composite Pass Shaders

## Purpose

This folder holds the shaders that merge the traced voxel image with everything Bevy draws itself, producing the final frame. The voxel renderer writes into a G-buffer and lit image of its own; this pass is the bridge to Bevy's render graph, where the atmosphere, UI, gizmos, particles and post effects live.

## Scope

- Combining the upscaled voxel image with Bevy's sky and procedural atmosphere, using the traced depth to decide what is sky.
- Distance fog and aerial perspective that hides transitions between detail levels at long range.
- Compositing of rasterized elements (UI, debug gizmos, particles such as dust) with correct depth occlusion against the voxel depth.
- Tone mapping, color grading and final output conversion for the display.
- Debug views: showing the G-buffer, mip levels, brick residency or feedback heat maps, when needed during development.

## Out of scope

- Tracing, lighting, reflections and upscaling. Those belong in the passes that produce the image this one consumes.
- UI layout and widgets. Those are Bevy UI in `World_Game`, with resources in `assets/ui`.
- Bevy's own post-processing shaders, which are part of Bevy.
- Render graph node registration, which is Rust in `World_Render`.

## Relationships

- Consumes output from `../upscale` and the depth written by `../primary`.
- Runs inside Bevy's Core3d graph; mirrors the composite pass module in `World_Render/src/passes`.
- Imports `../../common`.

## Rules

- This pass is the only place voxel output and Bevy-owned rendering meet; the other passes do not touch Bevy's targets directly.
- Output-resolution work only.
- Debug views are compiled behind conditionals and have no cost in normal builds.
- Must run on every supported tier with identical results apart from tier-specific quality options.
