# Player Models

## Purpose

This folder holds the voxel parts that make up the local player's body. The player has a full cartoon-proportioned body even in first person (bigger head and hands), because it is visible in shadows and in reflections. Each bone of the player rig owns one rigid voxel part, and those parts live here.

## Scope

- Voxel parts for the body: head, torso, upper and lower limbs, hands, feet, with overlap or joint-cap voxels so seams at joints are hidden.
- Both the `.vox` sources and the baked grids derived from them.
- Per-part metadata: pivot at the joint, voxel scale, and the bone name the part binds to.
- Variants of the same part (for example alternate clothing or skin palettes) when they share bone binding.

## Out of scope

- The skeleton itself (bone names, lengths, joint limits). That is a RON file in `assets/rigs`.
- Held items and tools. Those go in `../tools`.
- Creatures, NPCs and animals, which reuse the rig system later and get their own folders under `models/` when needed rather than being mixed in here.
- Animation clips and IK logic. Those are code and data in `World_Anim`.

## Relationships

- Parts are bound to bones by name from the player rig in `assets/rigs`; names here and there must match exactly.
- Loaded by `World_Anim`, which parents each part to its bone transform, and rendered through the box-march pass (`assets/shaders/passes/dynamic_box`).
- The head is hidden from the main first-person view only; it still renders in shadow and reflection passes.
- Built by `tools/Bake_Vox`.

## Rules

- Sources and baked files are tracked with Git LFS.
- Parts are rigid; deformation is achieved by overlap, not by per-frame rebuilding.
- Pivot points are at the joint, so rotating the bone transform rotates the part correctly.
- Material ids in palettes reference the material table in `assets/config`.
