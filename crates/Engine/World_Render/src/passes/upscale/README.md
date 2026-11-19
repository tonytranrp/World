# Upscale

## Purpose

Turns the noisy, internal-resolution lit image into the final output. It accumulates samples over time using motion vectors, then upscales to the native panel resolution. This is what lets the 2019 MacBook Pro floor render at about 720p internally and still fill a 3072x1920 panel, and lets the dev PC trade resolution for quality.

## Scope

- Temporal accumulation: reprojection with motion vectors from the G-buffer, history validation against depth and normals, clamping to limit ghosting, and reset on camera cuts or large edits.
- Sub-pixel camera jitter coordination with `primary`, so accumulation gains detail.
- Spatial upscaling from internal to output resolution, using a sharpening-aware filter that works on tier B compute.
- Optional tier-A upscalers (DLSS or DLSS-RR through the supported integration), selected by `tier` and falling back to the baseline path.
- Dynamic resolution control to hold the target frame time within the allowed internal resolution range.
- History textures and their lifetime tied to the view.

## Out of scope

- Producing the lit image: `lighting` and `reflections`.
- UI, tone mapping and post effects after the upscaled image: `composite`.
- G-buffer allocation: `gbuffer`, although this pass reads motion vectors and history from it.

## Relationships

Depends on `gbuffer` motion vectors, lit and reflection outputs, and `tier`. Feeds `composite`. Settings come from `assets/config` and the `--mac-floor` profile.

## Rules

- A tier B implementation always exists; vendor upscalers are optional additions.
- History is invalidated by anything that changes the world under it: large voxel edits, teleports, resolution changes.
- Output resolution is independent of internal resolution; neither is hard-coded.
- Accumulation must not smear moving dynamic objects; motion vectors from `dynamic_box` are required to be correct.
