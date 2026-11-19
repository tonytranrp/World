# Ray Traversal

## Purpose

This module answers geometric questions about the tree on the CPU: what does this ray hit first, is this segment unobstructed, which voxels does this box overlap. It is the production CPU traversal used by gameplay interaction, physics queries and streaming heuristics, and it carries the brute-force oracle against which tree traversal is verified.

## Scope

- Ray and segment types in voxel space and the entry from metre-space rays.
- Hierarchical DDA through the 64-tree that skips empty space using child masks, descends only into occupied children, and can stop at a chosen mip level for coarse queries.
- The hit record: voxel coordinate, material id, face normal, distance along the ray, and the mip level at which the hit was found.
- Occlusion queries for shadow and ambient-occlusion style tests, and box or region overlap queries.
- A brute-force dense DDA that steps voxel by voxel with no hierarchy, used only as a trusted oracle in tests.

## Out of scope

- Camera models, per-pixel ray generation, shading and image output: World_Cpu_Ref.
- GPU traversal: its WESL code lives in assets/shaders and is checked against the references.
- Swept shape casts and collision response: World_Physics.
- Tree storage itself: the tree module.

## Relationships

It depends on coordinates, materials and the tree's read interface. World_Cpu_Ref and World_Physics call into it, and the tests folder compares it with the brute-force oracle on random trees and random rays.

## Rules

- Traversal is exact: no tolerance-based misses at voxel boundaries. Ties are resolved by a documented, deterministic rule.
- Traversal only reads the tree and never allocates inside the hot loop.
- Hit results at a given mip level match what a coarser GPU ray must find, so mip semantics are defined here and mirrored elsewhere.
- Behavior near the world bounds and for rays starting inside solid voxels is specified and tested.
- The oracle is deliberately simple and is never optimized.
