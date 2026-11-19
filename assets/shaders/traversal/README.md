# Shader Traversal

## Purpose

This folder holds the single shared implementation of ray traversal over voxel data on the GPU. Every pass that casts a ray (primary, shadow, ambient occlusion, GI, reflection) calls the same code, so there is one place to get correctness and performance right and one behavior to compare against the CPU reference tracer.

## Scope

- Sparse 64-tree traversal: descending through nodes with 4x4x4 children and a 64-bit child mask, skipping empty space using bit operations.
- DDA stepping inside bricks and across node boundaries.
- Mip-level stop: distant rays terminate at a coarser level of the tree according to a per-pixel detail criterion.
- Brick-request feedback: when a ray reaches a node whose brick is not resident in the pool, it writes a request into the feedback buffer for the CPU to stream, then falls back to the coarser data it has.
- Box-march for dynamic objects: marching a ray through an object's own small dense grid in object space after a hit on its oriented box.
- Ray and hit result structures returned to callers.

## Out of scope

- Shading, lighting math and G-buffer writes. Those belong in `../passes`.
- Packing helpers, constants and material lookup. Those belong in `../common`.
- Streaming policy: how requests are processed, prioritized and evicted. That is CPU code in `World_Render`.
- The CPU tree itself. It is defined in `World_Core`, which is the source of truth.

## Relationships

- Imports `../common`; imported by every module in `../passes`.
- Memory layout must match `World_Core` node and brick definitions.
- Results must match `World_Cpu_Ref` exactly on small test scenes (checked by the render tests in the workspace `tests/`).

## Rules

- Traversal exposes a small API (cast a ray, get a hit or a miss with a request flag); callers must not depend on internal node layout.
- No pass-specific logic here.
- Feedback writes are bounded and deduplicated cheaply so an overloaded frame cannot flood the buffer.
- Behavior must be deterministic for a given input: no per-frame random choices.
- Any optimization is verified against the CPU reference before it is kept.
