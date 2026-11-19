# G-Buffer

## Purpose

Owns the intermediate render targets that connect geometry passes to lighting. Primary ray traversal and the dynamic-object box pass both write here, and lighting, reflections, temporal accumulation and upscale read from it. It is the contract between "what is visible" and "how it is lit".

## Scope

- Allocation and resizing of the G-buffer textures at the internal resolution: depth, normal, material id and motion vectors, plus any history targets needed for temporal reuse.
- Texture formats and encodings, such as octahedral normals and packed material ids, defined once so every pass agrees.
- Bind group layouts and shared WGSL/WESL struct definitions for G-buffer access, so shaders read and write through one description.
- Exporting depth into Bevy's depth texture so rasterized UI, gizmos and particles occlude correctly against voxel geometry.
- Per-view state tied to the camera: previous-frame targets, jitter, and resolution changes from `--mac-floor` or dynamic resolution.

## Out of scope

- Producing the contents: ray traversal is in `passes/primary`, dynamic objects in `passes/dynamic_box`.
- Consuming passes' own intermediate textures (shadow masks, probe data, reflection buffers): they live with their pass.
- Final screen-resolution targets and upscaling: `passes/upscale`.
- Material properties: the material table comes from World_Core, and the G-buffer stores only the material id.

## Relationships

Depends on Bevy render resources and tier-derived resolution settings from `tier`. Every folder under `passes` depends on it. `graph` orders the nodes that write and read it. The shaders that access it live in `assets/shaders/common`.

## Rules

- Both primary traversal and the box-march pass must write identical encodings so downstream passes cannot tell them apart.
- Memory scales with internal resolution only, never with world size.
- Layouts and formats are changed only here; shaders include the shared definitions rather than redeclaring them.
- Depth written for Bevy's depth texture must agree with the G-buffer depth to the precision of the format.
- Normals are computed at render time from neighboring voxels, not stored in the world.
