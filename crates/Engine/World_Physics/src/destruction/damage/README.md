# damage

## Purpose

This folder is the entry point of destruction: it converts physical events into voxel changes. A hit, a blast, or a tool strike becomes damage applied to voxels, and damage either removes voxels or weakens them so they fail more readily next time.

## Scope

- Damage events: a source, a position, an energy or impulse, and a shape of effect (point, sphere, line, swept volume).
- Sources of damage: contact impulses reported by the physics step, explosions, tool actions, and falling debris.
- Resolution against materials: comparing applied energy to each voxel's strength from the material table, with falloff by distance, deciding which voxels are removed and which merely accumulate damage.
- Per-voxel or per-region damage accumulation for weakened material.
- Issuing the resulting edits through World_Core's edit API, and notifying islands of the edited region and object so detection runs.
- Time-slicing: applying large damage volumes over several frames within the voxel budget.

## Out of scope

- Finding disconnected pieces: islands.
- Anchoring and falling terrain: anchors.
- Spawning bodies or particles: debris.
- Material values themselves: World_Core.
- Sources decided by gameplay, such as a weapon's stats: World_Game, which sends events here.

## Relationships

Depends on rapier (contact events), World_Core (edit API, material strength), and the parent destruction budget. Depended on by islands, which consume its edit notifications, and by World_Game and World_Sim, which raise damage events (for example, fire or stress failure).

## Rules

- Strength, hardness, and break behavior come only from the material table.
- Damage is applied to the authoritative voxel data via the edit API; no private copy.
- Resolution is deterministic for a given event and world state.
- Work per frame is capped by the shared voxel budget.
