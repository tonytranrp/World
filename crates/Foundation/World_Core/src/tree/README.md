# Tree

## Purpose

This folder is the sparse 64-tree: the data structure that stores the static world and the dense-versus-sparse hybrid that stores dynamic object grids. Each node has 4x4x4 children and a 64-bit child mask, which gives half the depth of an octree and lets traversal skip empty space with bit operations. Uniform interiors collapse into single nodes, so only surfaces cost memory. It is the single source of truth that the GPU brick pool mirrors.

## Scope

- The public tree type and its read interface: point lookup, region iteration, occupancy queries, neighbor access.
- Orchestration of the nested modules, each with a narrow job: node layout, allocation, mutation, and mip levels.
- Whole-tree operations that span those modules: bulk construction from a dense array, deep equality, structural statistics, and the canonical-form check.
- Support for dense small grids used by dynamic objects as a separate storage shape behind a shared read interface.

## Out of scope

- Edit semantics, batches and overlays: the edit module.
- Ray traversal algorithms: the ray module, which reads through this module's interface.
- Byte encoding: serialization.
- GPU node and brick pool packing and streaming: World_Render.
- Island detection and anything that decides what to remove: World_Physics.

## Relationships

The folder depends on coordinates and materials. Its subfolders form a stack: node is the base layout, alloc stores nodes, mutate writes through alloc, mip derives coarse levels from the result. Edit, ray, serialization, World_Gen and World_Cpu_Ref depend on this module's public interface.

## Rules

- The tree is canonical: the same voxel content always produces the same structure, so equality, hashing and golden tests are meaningful.
- Edits cost O(log n) by path copying and never touch unrelated subtrees.
- Memory is indices, not pointers, so storage can be mirrored, compacted and serialized.
- Reading never mutates, and readers see a consistent tree.
- Layout facts the GPU depends on live in node and are expressed through the shared constants.
