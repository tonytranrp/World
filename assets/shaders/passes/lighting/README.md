# Lighting Pass Shaders

## Purpose

This folder holds the shaders that light the frame after the G-buffer is built. Nothing is baked, because everything in the world can be destroyed, so all lighting is computed at runtime by tracing through the same voxel structure the primary pass uses. The look is painterly: saturated per-voxel color, soft bounce light and gentle fog, not photoreal materials.

## Scope

- Sun shadows: shadow rays from G-buffer positions toward the sun.
- Ambient occlusion: short rays that darken crevices, matter most at close range.
- Global illumination: a world-space radiance cache of irradiance probes per brick, updated incrementally, fed by the sun and emissive voxels such as lamps and lava. Includes temporal accumulation and denoising.
- Emissive handling: light-emitting materials from the material table injecting into the cache.
- The quality ladder for different tiers (cache update rate, ray counts), so the baseline tier B runs with software GI and the optional tier A extras are conditional modules.

## Out of scope

- Reflections. Those are in `../reflections`.
- Final temporal upscaling of the whole image, in `../upscale`.
- Tree traversal, in `../../traversal`.
- Sky and atmosphere, which come from Bevy's procedural atmosphere in the composite stage.
- Light parameters and quality knobs, which are RON in `assets/config`.

## Relationships

- Reads the G-buffer produced by `../primary` and `../dynamic_box`; imports `../../traversal` and `../../common`.
- Mirrors the lighting pass module in `World_Render/src/passes`.
- Probe storage is tied to the brick pool, so brick residency rules in `World_Render` define probe lifetime.

## Rules

- No lighting state is stored on disk or baked.
- Cost per frame is bounded and adjustable through config; cache updates are spread over frames.
- Tier B code path is the baseline; hardware-RT extras sit behind conditional compilation.
- Output stays stable (no visible boiling) at the target frame rates.
