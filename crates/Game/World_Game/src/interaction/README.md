# Interaction

## Purpose

The interaction domain is where the player's intent touches the world. It finds what the player is pointing at, decides what happens to it, and applies the result as voxel edits and physics requests. It is the single gameplay gateway into World_Core edits and World_Physics.

## Scope

- Picking and targeting: casting rays from the eye or tool muzzle through the CPU 64-tree and against dynamic bodies, producing hit records (position, normal, material, object).
- Damage resolution: converting a tool's effect (point, sphere, cone, cut plane, blast) into per-voxel damage using material strength, producing removed or weakened voxels.
- Edit application: building World_Core edit batches and submitting them, so the physics layer can run island detection on the edited region.
- Impulse and force requests to bodies hit by tools.
- Highlighting and focus state for targeted objects, as data for rendering and the UI.

## Out of scope

- Tool behavior, charging and cooldowns belong in `tools`.
- Island detection, splitting into bodies and debris-to-particle conversion belong in World_Physics.
- Voxel storage, edit primitives and the traversal algorithm belong in World_Core.
- Visual and audible feedback belongs in `effects` and `audio`.

## Relationships

Consumes damage and edit requests from `tools` and `sandbox`, and eye and look data from `player`. Depends on World_Core and World_Physics. Emits hit and break events for `effects`, `audio` and `ui`. Writes to `save` indirectly by recording edits in the overlay.

## Rules

- All world mutation passes through World_Core edit operations; no direct tree writes.
- Edits are batched per frame and bounded; large destruction is time-sliced rather than applied in one frame.
- Hit tests use the same traversal semantics as the CPU reference, so picking matches what is rendered.
- Damage depends on material table values only.
- Systems stay in `Update` where possible, with stable component layouts.
