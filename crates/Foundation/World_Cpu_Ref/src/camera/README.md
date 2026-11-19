# Camera

## Purpose

This module turns a camera description and a pixel into a ray. The accuracy of the whole comparison against the GPU depends on both sides starting from identical rays, so ray generation is specified exactly here and kept simple.

## Scope

- The camera description: position, orientation, vertical field of view, near and far limits, and the internal render resolution.
- Pixel-to-ray mapping: pixel center convention, normalized device coordinates, direction construction and normalization, in a fixed order of operations.
- The previous-frame camera, used to compute motion vectors for the reference G-buffer.
- Conversion from metres to the voxel-space rays used by traversal.
- Tile and beam grouping of pixels, where the GPU processes 8x8 tiles, so tests can compare per-tile behavior.

## Out of scope

- Free-look controls, view bobbing, player rigs and first-person camera behavior: World_Game and World_Anim.
- Projection used by Bevy's rasterized passes: World_Render.
- Anti-aliasing, jitter and temporal accumulation patterns, which belong to later render stages.
- Tracing the rays: the trace module.

## Relationships

It depends on World_Core coordinate types. The trace module consumes its rays; the output module needs the camera to define depth and motion vectors. World_Render's tests pass the same camera parameters to both this module and the GPU.

## Rules

- Ray generation is deterministic and independent of the order pixels are processed.
- The same camera parameters must produce the same ray on the GPU, to the last bit where operations are exactly specified, so the order of floating-point operations is documented.
- Handedness, axis directions and up vector conventions are stated once in this module and referenced elsewhere.
- Degenerate cameras (zero field of view, non-finite values) are rejected explicitly.
- Rays carry enough information (origin, direction, inverse direction, limits) that traversal never recomputes them.
