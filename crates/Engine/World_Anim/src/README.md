# World_Anim source

## Purpose

This folder is the source root of the animation crate. It exposes the crate's one Plugin and prelude and arranges the module folders that make up the rig, animation, IK, and ragdoll stack. It sets the order in which those stages run each frame.

## Scope

- The crate's single public Plugin entry point and its prelude, re-exporting rig handles, part components, IK targets, and ragdoll triggers.
- Schedule wiring: the ordering of the pipeline per frame. Rig and part setup first, then AnimationGraph sampling, then procedural motion (IK, look-at, springs), then transform propagation. Ragdoll switching runs in the fixed physics step, with a handoff that blends from animated to simulated.
- Crate-level shared types used by several modules, such as the rig-state component and animation-mode enum (animated, procedural, ragdoll).
- Module declarations for rig, parts, import, ik, procedural, and ragdoll.

## Out of scope

- Any detailed behavior: each concern lives in its module folder.
- Direct physics calls beyond World_Physics' public types.
- Voxel and material definitions: World_Core.
- Tests: the tests folder beside this one.

## Relationships

Depends on every module below it, plus World_Physics and World_Core. Depended on, through the prelude, by World_Game and the app. The modules relate in a layered way: rig defines the skeleton, parts attach voxel objects to it, import supplies the parts, ik and procedural modify poses, ragdoll hands bones to physics.

## Rules

- One Plugin, one prelude; adding a module never adds a new entry point.
- Stage order is declared with system sets and must hold under randomized schedule ordering.
- Dependencies between modules point from the procedural and physical layers down to rig and parts, never the reverse.
- The crate does not import the physics engine directly; it uses World_Physics' types.
