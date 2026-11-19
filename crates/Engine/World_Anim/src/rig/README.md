# rig

## Purpose

This folder defines what a skeleton is: the bone hierarchy, the lengths and proportions, and the joint limits. It is the data model and loader for rigs, and the starting point for every other animation module. The same system serves the local player first and creatures and NPCs later.

## Scope

- The rig description: bone names, parent-child hierarchy, bone lengths, rest poses, joint types, and joint limits, plus named attachment points such as eyes, hands, and feet.
- The RON rig asset and its loader, with validation (no cycles, unique names, sane limits).
- Hot reload: when the asset changes on disk, the live skeleton updates in place without a recompile.
- Spawning a rig into the ECS: a bone entity hierarchy whose Transforms Bevy's AnimationGraph can drive, addressable by bone name.
- Cartoon proportion parameters, such as head and hand scale, as data on the rig.
- Player-specific rig facts: eye bone placement for the camera, and which part is the head for first-person hiding.
- Rig variants and shared definitions so a creature can reuse a humanoid template.

## Out of scope

- The voxel content of each part: parts and import.
- IK, look-at, springs, procedural gait: ik and procedural.
- Simulation of bones as bodies: ragdoll.
- Animation clips and graph authoring beyond naming bones they target.

## Relationships

Depends on World_Core constants and Bevy assets. Depended on by parts, ik, procedural, and ragdoll, all of which address bones through this module's types. It depends on none of them.

## Rules

- Bone names are the stable public key; clips, IK targets, and ragdoll bodies refer to bones by name.
- Rigs are data: no proportion, limit, or hierarchy is hard-coded in Rust.
- Reloading must preserve entity identity where bones persist.
- A rig knows nothing about physics engines.
