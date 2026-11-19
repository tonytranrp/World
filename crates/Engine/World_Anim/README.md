# World_Anim

## Purpose

World_Anim gives voxel characters a body that moves. It defines how a skeleton of bones maps onto rigid voxel parts, drives those parts with animation, adds procedural motion on top, and hands the same parts to physics for ragdoll and dismemberment. The local player's rig comes first; creatures, NPCs, and animals reuse the same rig system.

## Scope

- Rigid part per bone: each bone owns one rigid voxel part, so a character is a hierarchy of separate voxel objects. Seams at joints are hidden by overlapping parts and joint-cap voxels.
- Rig definition: a hot-reloadable RON asset describing bone names, lengths, hierarchy, and joint limits, with cartoon proportions (a larger head and hands). Editing the file updates the live rig without a recompile.
- Animation driving: Bevy's AnimationGraph animates bone Transforms like any entity hierarchy.
- Procedural layer: two-bone foot IK that plants feet on uneven voxel ground, look-at, procedural walk cycles for creatures, and spring-driven secondary motion.
- Ragdoll: a physics mapping where every part becomes a body and every bone a joint, so limbs can break off.
- Part authoring and import from MagicaVoxel .vox files.
- Player presentation rules: the camera sits at the eyes, the head part is hidden from the main view only, and the full body stays visible in shadows and reflections.

## Out of scope

- Vegetation sway: a shader ray-space warp in World_Render, not voxel data movement.
- Shell-mesh ray deformation for faces and creatures: not part of this crate's rigid-part approach.
- Rigid body simulation and joints: World_Physics.
- Voxel storage and rendering: World_Core and World_Render.
- Creature behavior and AI: World_Game.

## Relationships

Depends on World_Core (voxel parts), World_Physics (bodies, joints, ground queries), and Bevy. Depended on by World_Game and the app. World_Render reads the part objects but does not depend on this crate.

## Rules

- A bone always corresponds to exactly one part and, in ragdoll, exactly one body.
- Rig data is data, not code: proportions, limits, and names live in the asset.
- Procedural layers adjust animation output and never alter voxel contents.
- Heavy per-frame work is time-sliced, and the crate is tested headlessly.
