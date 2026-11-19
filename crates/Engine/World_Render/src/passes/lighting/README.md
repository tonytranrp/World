# Lighting

## Purpose

Category folder for lighting the G-buffer. Light in this game cannot be baked because everything can be destroyed, so it is computed at runtime by tracing through the same voxel structure. The look is painterly: saturated per-voxel color, soft bounce light and gentle fog, rather than photoreal materials.

## Scope

- Orchestration of the lighting stages and the combination of their results into one lit color target.
- The progression of techniques: v1 sun shadow rays and ambient occlusion (`shadow`, `ao`), v2 world-space radiance cache global illumination (`gi`), v3 optional Solari and DLSS-RR on tier A.
- Shared lighting inputs: the sun and sky parameters, emissive voxel sources such as lamps and lava, fog, and tuning from `assets/config`.
- The tier-A lighting extras for triangle content, gated by capability, and their fallback to the software path elsewhere.

## Out of scope

- Primary visibility: `primary` and `dynamic_box`.
- Reflections of the voxel world: `reflections`.
- Temporal accumulation of the final image and upscaling: `upscale`.
- Material data: World_Core. Atmosphere and sky rendering: Bevy.

## Relationships

Depends on `gbuffer`, on `brick_cache` for traversal through the pool, and on `tier` for extras. Subfolders `shadow`, `ao` and `gi` are its stages. `upscale` consumes its output. Shaders mirror this layout under `assets/shaders/passes/lighting`.

## Rules

- Tier B must produce complete lighting with software GI; tier A extras only add.
- All rays use the shared traversal modules, so lighting sees the same world the primary rays saw, including dynamic boxes.
- Nothing is precomputed or stored in world files; lighting state is a cache that can be rebuilt.
- Stages communicate through well-defined textures and buffers, not through shared mutable shader state.
- Cost must scale down on `--mac-floor`: fewer rays, lower update rates.
