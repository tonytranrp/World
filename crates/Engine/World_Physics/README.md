# World_Physics

## Purpose

World_Physics is the single crate that connects the voxel world to a rigid-body physics engine and owns everything that makes that world physically react: colliders built from voxels, rigid bodies for dynamic objects, destruction, debris, and the player's character controller. The physics engine (Rapier through bevy_rapier3d) is an implementation detail confined to this crate.

## Scope

- Stepping physics at a fixed 60 Hz in `FixedUpdate`, with transform interpolation for rendering. The game is single-player, so Rapier runs in its parallel mode without enhanced determinism.
- Two resolutions: render voxels at 1/64 m and collision voxels at 1/16 m, so collision handles 64 times fewer voxels. The collision grid size is a tuning knob read from World_Core constants.
- Static world colliders built in chunks (about 4 m), only near awake bodies and the player, and rebuilt when the underlying voxels are edited.
- Dynamic objects as rigid bodies with voxel colliders over their own dense grids, with mass and inertia summed from per-voxel material density.
- Destruction: damage removes or weakens voxels, union-find or flood fill runs over the edited object, and each disconnected island becomes a new body or, if tiny, debris particles.
- A kinematic capsule character controller with move-and-slide, about 0.3 m step-up, and ground snapping.
- Mirroring the physics world into the ECS.

## Out of scope

- Voxel storage, edit API, materials, and constants: World_Core.
- Drawing bodies or particles: World_Render.
- Rigs, IK, and ragdoll mapping logic: World_Anim, which consumes bodies and joints from here.
- Structural stress, sand, fire, fluids: World_Sim.
- Gameplay decisions such as what deals damage: World_Game.

## Relationships

Depends on World_Core (voxels, edit API, constants, material table) and Bevy. Depended on by World_Anim (ragdoll), World_Sim (island detection, reacting to edits), and World_Game. The submodules depend downward: destruction and character build on colliders; everything touching the engine goes through the rapier module.

## Rules

- Rapier is named only inside the rapier module. All other modules speak World_Physics' own types.
- Active bodies are hard-capped (about 500); sleeping stays on; islands under about 64 voxels become GPU particles, not bodies.
- Voxel-count-proportional work is time-sliced to N voxels per frame.
- Material properties come only from the World_Core table.
- Behavior is verified by deterministic replay and regression scenes with recorded hashes.
