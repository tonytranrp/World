# parts

## Purpose

This folder is where bones get bodies. Each bone owns one rigid voxel part, a small dense voxel grid rendered and simulated as a separate object. This module defines parts, attaches them to rig bones, and keeps them following their bones.

## Scope

- The part model: a dense voxel grid in the bone's local space, with its own origin, pivot, and resolution. Hands and held tools may use finer resolution than body parts, since they fill the screen.
- Attaching a part to a bone and keeping the part's object transform synchronized with the bone's animated transform each frame.
- Joint-cap voxels and deliberate overlap between neighboring parts, which hide seams when bones rotate.
- Visibility rules per part, such as the head hidden from the main camera view only while still casting shadows and appearing in reflections.
- Part variants and swapping: equipment, clothing, and tools as parts parented to a bone or attachment point.
- Registering parts with the renderer's dynamic-object path as oriented boxes over their grids.
- Detachment: converting a part into an independent object when a limb breaks off.

## Out of scope

- Reading .vox files: import.
- Skeleton structure and joint limits: rig.
- Mass and colliders for parts as bodies: World_Physics colliders; ragdoll in this crate decides when parts become bodies.
- How boxes are drawn: World_Render.

## Relationships

Depends on rig, World_Core's dense grid and material types, and import for authored data. Depended on by ragdoll (one body per part), ik and procedural (via bone transforms, not parts directly), and World_Render, which draws the objects registered here.

## Rules

- One bone, one part; compound visuals are made from attachments, not by sharing a bone.
- Part voxel data is never edited by animation; only transforms change.
- Part transforms derive from bones, never the reverse, except during ragdoll.
