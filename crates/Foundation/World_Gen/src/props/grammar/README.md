# Prop Grammars

## Purpose

This module generates the structure of props that need rules rather than a single formula: the branching of trees, the layout of ruins, the growth of vines. It outputs a description made of distance field primitives and transforms, which the sdf module evaluates and the voxelize module rasterizes.

## Scope

- L-system definitions and their deterministic expansion: symbols, production rules, stochastic rules driven by a seeded stream, and interpretation into a skeleton of segments with radii.
- Space colonization for natural tree crowns: attraction points, node growth and branch thickness from the pipe model.
- Shape grammars for ruins and buildings: modular pieces, adjacency rules, wall and arch generation.
- Conversion of skeletons and layouts into SDF primitives (capsules, boxes, blended unions) with material tags such as bark, leaf and stone.
- Complexity bounds: limits on depth, segment count and instance count so a prop's cost is predictable.

## Out of scope

- The distance field primitives and combinators: sdf.
- Rasterization to voxels and resolution handling: voxelize.
- Where props are placed: the props module above.
- Terrain shaping and general noise.

## Relationships

It depends on noise for seeded streams and on sdf for the primitive types it emits. The props module calls it to build structural descriptions, and voxelize consumes what it ultimately produces through the sdf representation.

## Rules

- Expansion is deterministic: the same seed and parameters produce the same skeleton in the same order, with no dependence on hash-map iteration.
- Growth algorithms use fixed iteration orders and tie-break rules, and bounded loops.
- Output is resolution independent. Nothing here refers to voxel size.
- Random choices draw from named sub-streams so adding a rule does not shift the randomness of existing ones.
- Material tags are symbolic; mapping them to material ids is a later step.
