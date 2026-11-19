# World_Physics source

## Purpose

This folder is the source root of the physics crate. It defines the crate's public surface and arranges the module folders that implement it. Its job is composition: expose one Plugin and one prelude, register the physics schedule, and decide the order in which the submodules run each fixed step.

## Scope

- The crate's single public Plugin entry point and its prelude, re-exporting only the types other crates may use (body handles, damage events, controller components, configuration resources).
- Schedule wiring: system sets inside `FixedUpdate` that order collider building, the physics step, ECS mirroring, damage application, island detection, and debris emission, plus the interpolation that runs after the step.
- Crate-level resources and events shared by more than one module, such as the physics configuration and the time-slice budget handle, when no single module owns them.
- Module declarations for the folders beside it: rapier, colliders, destruction, and character.

## Out of scope

- Any concrete behavior: collider building, island detection, damage, and character movement each live in their own module folder.
- Any direct use of the physics engine API: only the rapier module touches it.
- Voxel and material definitions, which belong to World_Core.
- Tests and benchmarks, which live in the sibling tests and benches folders.

## Relationships

Depends on every module folder below it and on World_Core and Bevy. Depended on, through the prelude, by World_Anim, World_Sim, and World_Game. The module folders do not depend on this folder's internals, only on shared types it re-exports.

## Rules

- The public surface stays small: one Plugin, one prelude. Adding a module does not add a new public entry point.
- Module dependencies point one way: character and destruction depend on colliders; colliders depend on rapier; nothing depends on the crate root's wiring.
- System ordering is declared explicitly with system sets and must be correct under randomized schedule ordering.
- No module imports the physics engine crate directly, only through the rapier module.
