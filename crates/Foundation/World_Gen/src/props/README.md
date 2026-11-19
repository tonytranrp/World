# Props

## Purpose

This module makes the discrete objects in the world: rocks, trees, ruins and other structures. They are described as signed distance field graphs and grammars rather than as voxel lists, so they are independent of resolution. A prop can be voxelized at whatever resolution the requested level of detail needs, which means level of detail for props comes for free and one description serves close-up and distant views.

## Scope

- Placement: deciding where props occur, using deterministic distribution (blue-noise or cell-jittered sampling), biome and slope rules, and spacing so props do not conflict.
- Prop definitions: a prop is a deterministic recipe from a seed to a distance field and a material assignment.
- Orchestration of three nested concerns: sdf (primitives and graphs), grammar (procedural structure such as L-systems and space colonization), and voxelize (turning a distance field into voxels at a resolution).
- Bounds: each prop reports a conservative bounding box so chunk code knows which chunks it touches.
- Variation: per-instance seed variation, scale, rotation in a deterministic and discrete form.

## Out of scope

- Terrain and caves: continental and regional.
- Surface layering of terrain: surface.
- Authored voxel models and baked assets: Bake_Vox and World_Core serialization.
- Animated or physical objects: World_Anim and World_Physics.
- Gameplay items and loot.

## Relationships

It depends on noise, on continental data for placement, and on World_Core materials. Chunk calls it to gather props that intersect a chunk. The three nested folders are used only through this module.

## Rules

- A prop's appearance is a pure function of its seed, its parameters and the resolution requested; coarser resolution never changes silhouettes beyond sampling error.
- A prop spanning several chunks produces identical voxels no matter which chunk requests it.
- Placement never depends on the order props are evaluated.
- Props reference materials by id and do not define materials.
