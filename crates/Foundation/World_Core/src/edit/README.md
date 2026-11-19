# Edits

## Purpose

This module expresses changes to the world as data and applies them to the tree. Player digging, destruction and generator output all become edit operations. It also defines the overlay: the sparse record of everything that differs from the base world regenerated from the seed, which is the only thing saves store.

## Scope

- Primitive edit operations: set a voxel, clear a voxel, fill a box, carve or fill a region described by a shape, replace one material with another.
- Batches of edits applied atomically in a deterministic order.
- The dirty set produced by applying edits: touched bricks and nodes, which drives GPU brick upload, mip refresh and physics collider rebuilds.
- The overlay: per-chunk sparse edit records layered over a generated base, with merge, compaction and the rule for resolving later edits over earlier ones.
- Edit journals suitable for undo and redo within a session.

## Out of scope

- The low-level tree write path, path copying and node allocation: the tree mutate and alloc modules.
- Deciding why something is edited (damage models, tools, explosions, island detection): World_Physics, World_Sim and World_Game.
- Byte formats for storing overlays: the serialization module.
- Generating the base world: World_Gen.

## Relationships

It depends on coordinates, materials and the tree. Serialization stores its overlay types. World_Physics and World_Render consume the dirty sets it returns. World_Gen produces voxels that the overlay is applied over.

## Rules

- Applying the same batch to the same tree yields the same tree and the same dirty set, on every machine.
- The overlay plus the regenerated base fully reconstructs the world; nothing derivable from the seed is ever stored in it.
- Edits are idempotent where defined, and a no-op edit reports an empty dirty set.
- Edits never allocate outside the fixed world bounds; out-of-range requests fail explicitly.
- All edit paths are checked against a dense reference array by property tests.
