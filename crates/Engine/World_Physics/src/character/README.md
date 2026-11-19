# character

## Purpose

This folder implements the player's character controller: how a controllable body moves through a world made of 1/64 m voxels without catching on every tiny stair-step. It is a kinematic capsule driven by move-and-slide, not a dynamic rigid body.

## Scope

- The kinematic capsule: its dimensions, its collision shape, and its relation to collision-grid terrain at 1/16 m.
- Move-and-slide: casting the capsule along the desired motion, resolving contacts, and sliding along surfaces over several iterations until the motion is consumed.
- Step-up of about 0.3 m, so stairs, ledges, and rough voxel terrain are climbed instead of blocking.
- Ground snapping, so walking downhill or off small drops keeps the capsule attached, and a ground-state model (grounded, airborne, sliding) with slope limits.
- Gravity, jumping, and velocity integration for the controller, plus pushing dynamic bodies it touches and being carried by moving ground.
- Producing the controller's resulting motion for the fixed step and exposing it for interpolation and for camera and rig systems to read.

## Out of scope

- Input mapping and gameplay tuning such as sprint rules: World_Game.
- The visible body, IK foot placement, and animation: World_Anim.
- Static terrain collider construction: colliders/chunk.
- The shape-cast and contact primitives: rapier.

## Relationships

Depends on colliders (the terrain it moves over), rapier (casts and contact queries), and World_Core constants. Depended on by World_Anim, which reads ground and velocity state to drive IK and walk cycles, and by World_Game. It has no dependency on destruction.

## Rules

- The controller moves in the fixed 60 Hz step only; rendering uses interpolation.
- Step height, slope limit, and snap distance are data-driven, not hard-coded, and chosen against the 1/16 m collision grid rather than the render grid.
- Behavior must be stable on rough ground: no jitter, tunneling, or catching on voxel steps.
- The controller never writes to voxels.
