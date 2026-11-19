# rapier

## Purpose

This is the only place in the entire workspace that names Rapier or bevy_rapier3d. It is an adapter layer: World_Physics' own physics concepts (body, collider, joint, contact, query, step) are translated here into calls on the Rapier world, and Rapier results are translated back. Swapping to Avian must mean rewriting this folder and nothing else.

## Scope

- Creating, updating, and removing Rapier rigid bodies, colliders, and joints from World_Physics' engine-neutral descriptions.
- Constructing Rapier voxel shapes from collision-grid data, and updating them incrementally when voxels change.
- Configuring the Rapier pipeline: the fixed 60 Hz timestep, parallel solver mode, solver iterations, sleeping thresholds, and collision groups.
- Scene queries (raycasts, shape casts, overlaps) and the shape-cast and contact primitives the character controller needs.
- Contact and collision event translation, including contact force magnitudes that destruction uses as damage input.
- Mapping between ECS entities and Rapier handles, and mirroring the physics world into the ECS.

## Out of scope

- Deciding what to collide with or when to build colliders: colliders.
- Damage, islands, anchors, debris: destruction.
- Character movement logic: character. This folder provides only the primitives.
- Voxel and material data: World_Core.

## Relationships

Depends on bevy_rapier3d and World_Core's constants. Depended on by colliders, destruction, and character, which call it through World_Physics' own types. No module outside this crate may import anything from here, and no other crate may depend on Rapier.

## Rules

- No Rapier type appears in any public signature outside this folder. Handles are wrapped in crate-owned types.
- Translation is lossless for what the rest of the crate uses; anything Rapier-specific stays internal.
- Rapier's parallel mode is used without enhanced determinism; this layer does not promise cross-machine reproducibility.
- Panics are avoided: a stale handle is an error value, not a crash.
