# World_Game

## Purpose

World_Game (package `world_game`) is the gameplay crate: the fast-changing systems that turn the engine into a first-person destruction sandbox. It is the most hot-patched crate in the project, iterated live with `dx serve --hot-patch`, so its structure is shaped by that workflow. It exposes a single game Plugin and a prelude; the application binary adds the plugin and nothing else from this crate.

## Scope

- Player: first-person controller glue, the full body, the camera placed at the eyes, input mapping.
- Tools: hand tools, held items, explosives, and the cutting, digging and destroying tools.
- Interaction: ray picking, targeting, and applying damage and voxel edits through World_Core and World_Physics.
- Sandbox: scenario spawning, test arenas, creative-mode controls.
- Creatures: NPCs and animals built on the World_Anim rig system, with behavior and AI.
- Effects: gameplay side of dust, debris particles and impact feedback.
- Audio: material-driven break and impact sounds.
- UI: HUD, menus and debug overlays.
- Save: orchestration of saving and loading the edit overlay and player state.
- Integration tests that run the game plugin headlessly.

## Out of scope

- Voxel storage, edit primitives, the save file format and materials belong in World_Core.
- Rendering, brick streaming, the G-buffer and lighting belong in World_Render.
- Rigid bodies, colliders, island splitting and the character controller backend belong in World_Physics.
- Rig definitions, IK and ragdoll mapping belong in World_Anim.
- Fire, sand, fluid and stress simulation belong in World_Sim.
- Backend selection, settings and startup belong in `app/`.

## Relationships

Depends on World_Core, World_Physics, World_Render, World_Anim, World_Sim and, for sandbox scenarios, World_Gen, always through their plugins and preludes. The only dependent is `app/`, which assembles the plugin set. Each folder inside the source tree is one gameplay domain with its own sub-plugin, registered by the crate's single top-level Plugin.

## Rules

- Systems run in `Update` (or `FixedUpdate` where physics coupling requires it) so they patch live.
- Avoid struct layout churn on components and resources that exist while the game runs; adding fields to hot types can crash a patched process. Prefer new components over changing existing ones.
- Tunables live in RON assets or the inspector, not in recompiled constants.
- Gameplay mutates the world only via World_Core edit operations and World_Physics requests.
- Domains do not reach into each other's internals; they communicate through events and shared components.
- Systems must be order-independent unless explicitly constrained, because tests run with randomized schedules.
