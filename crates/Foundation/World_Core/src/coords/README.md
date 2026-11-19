# Coordinates

## Purpose

This module defines every way of addressing space in the world and the conversions between them. It is the one place that knows how a position in metres becomes a voxel, a collision cell, a brick, a chunk, or a path of child indices down the 64-tree.

## Scope

- Integer voxel coordinates at 1/64 m, the canonical address of any voxel in the fixed large world.
- Coarse collision-cell coordinates at 1/16 m, and the exact mapping from collision cells to the voxels they cover.
- Brick and chunk coordinates, and the origin and extent of each in voxel units.
- Tree paths: splitting a voxel coordinate into per-level child indices (0 to 63, from the 4x4x4 fan-out), and the inverse. Linear and Morton-style indexing inside a 4x4x4 node.
- Axis-aligned boxes and ranges in voxel space, intersection and iteration helpers.
- The conversion boundary to f32 metres, including the bounds of the fixed world so precision stays near 2 mm.
- Face and direction enums used for neighbor lookup and ray-hit normals.

## Out of scope

- The constants themselves (voxel size, collision scale, brick dimension): they live once in the crate source root and this module consumes them.
- Transforms, rotations and matrices for rendering or physics: Engine crates.
- Tree storage and traversal logic: the tree and ray modules.

## Relationships

A leaf module: it depends only on the shared constants. Tree, edit, ray and serialization all build on it, as do World_Gen and World_Cpu_Ref.

## Rules

- Integer-only for all addressing. Floats appear only in explicit metre-to-voxel conversions.
- Division and shifting use floor semantics so negative coordinates map consistently; this is property tested against a naive reference.
- Conversions are exact inverses where defined, and lossy conversions are named as such.
- Types are small, Copy, hashable and ordered, with stable hashing for golden tests.
- No coordinate type may silently mix units; voxel, cell, brick and chunk coordinates are distinct types.
