# Rigs

## Purpose

This folder holds rig definitions: the skeletons that organize voxel parts into animated characters. A rig is data, written in RON, so it hot-reloads. Editing a rig file while the game runs changes the skeleton on screen without a recompile.

## Scope

- Bone hierarchy and bone names.
- Bone lengths and rest-pose offsets, in metres.
- Joint types and limits (rotation ranges, twist limits) used by IK clamps and by the ragdoll mapping.
- The binding from each bone to the voxel part it owns, referenced by name from `assets/models`.
- Rig-level settings such as eye position for the first-person camera, foot-contact points for IK and which parts are hidden from the main view.
- The player rig first; creatures, NPCs and animals later reuse the same format.

## Out of scope

- The voxel parts themselves. Those live in `assets/models`.
- Animation clips and the animation graph. Clips are authored content for Bevy's AnimationGraph and the graph wiring is code and data in `World_Anim`.
- IK and ragdoll algorithms. Those are code in `World_Anim` and `World_Physics`.
- Gameplay parameters such as walk speed. Those go in `assets/config`.

## Relationships

- Loaded and interpreted by `World_Anim`, which spawns one entity per bone and attaches parts to them.
- Joint limits are also read when building ragdoll bodies and joints through `World_Physics`.
- Part names must match the models they reference.
- Camera setup in `World_Game` reads the eye anchor.

## Rules

- RON text only, with units in comments or field names.
- A rig must be valid on its own: bone names unique, parents defined before children, every limit within sane ranges. Loading rejects invalid rigs and keeps the previous good one when hot-reloading.
- Rigs describe structure, not appearance: skins and palettes belong with the models.
- Cartoon proportions (large head and hands) are expressed in the lengths and parts, not special-cased in code.
