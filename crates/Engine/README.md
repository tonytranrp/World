# Engine

## Purpose

This folder holds the Bevy-facing engine crates of the workspace: the crates that take the Bevy-free data and algorithms from Foundation and turn them into running systems inside an `App`. Each crate here owns one engine concern (rendering, physics, animation, extra voxel simulation), exposes exactly one public Plugin entry point plus a prelude, and is built from a small number of crates with deep module folders rather than many tiny crates.

## Scope

Four sibling crates live here, each in a Pascal_Case folder with a snake_case package name:

- **World_Render** (`world_render`): the ray-traced voxel renderer. Compute traversal of the 64-tree into a G-buffer, the ray-guided GPU brick cache, the box-march pass for dynamic objects, lighting, reflections, and capability-tier selection. It mirrors the CPU tree to the GPU.
- **World_Physics** (`world_physics`): the bridge between World_Core voxel data and the physics engine. Fixed 60 Hz stepping, voxel colliders at 1/16 m, rigid bodies for dynamic objects, destruction and island detection, debris, and the character controller.
- **World_Anim** (`world_anim`): voxel rigs, bone-driven rigid parts, procedural IK, and the ragdoll mapping onto physics bodies.
- **World_Sim** (`world_sim`): extra voxel simulations beyond rigid bodies: structural stress, sand, fire, and fluids, all time-sliced under one shared budget.

## Out of scope

- Voxel storage, the 64-tree, the material table, shared constants, edits and serialization: those are World_Core in Foundation.
- Procedural generation and the CPU reference implementations: World_Gen and World_Cpu_Ref in Foundation.
- Gameplay rules, game state, and anything that is hot-patched often: World_Game under Game.
- Backend selection, CLI flags, and settings: the app binary.
- Offline baking and conversion: the tools folder.

## Relationships

Dependency direction is one way: Foundation <- Engine <- Game <- app. Every Engine crate may depend on Foundation crates and on Bevy; Foundation never depends on anything here.

Among the siblings, the dependencies are deliberately thin. World_Anim depends on World_Physics for ragdoll bodies and joints. World_Sim depends on World_Physics for island detection and for reacting to edits. World_Render does not depend on physics, animation, or simulation; it reads shared voxel and object data and mirrors it to the GPU. World_Game composes all four.

## Rules

- One public Plugin and one prelude per crate. Everything else is private to the crate or exposed only through the prelude.
- The CPU 64-tree is the single source of truth. Engine crates edit it only through World_Core's edit API and never keep a second authoritative copy of voxel data.
- The physics engine (Rapier through bevy_rapier3d) is named only inside World_Physics. No other Engine crate imports it, so replacing it with Avian touches one crate.
- Anything that touches many voxels per frame is time-sliced under an explicit budget so no system can stall a frame.
- Shared constants (voxel size 1/64 m, collision grid 1/16 m, brick dimension) and material properties come from World_Core, never redefined locally.
- Every crate is testable headlessly with a `MinimalPlugins` App, with schedule randomization enabled, and compares against a simpler reference implementation.
