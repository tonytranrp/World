# Pool

## Purpose

Owns the fixed-size GPU storage that mirrors the CPU 64-tree: a node pool holding tree nodes (child masks and child indices) and a brick pool holding leaf voxel data and palette indices. It is the structure the traversal shaders actually walk.

## Scope

- Allocation of the node pool and brick pool as GPU storage buffers (or textures where the adapter requires), sized from tier and settings.
- The GPU layout of a node and a brick: 64-bit child masks, child offsets into the pool, palette references, and mip-level markers, derived from World_Core's constants.
- Slot management: a free list of node and brick slots, the mapping from CPU tree node identity to GPU slot, and the residency metadata the shaders need to detect a missing child.
- Residency of coarse mips alongside full-resolution bricks, and the root and metadata buffers that point shaders at the pool.
- Bind group layouts and shared shader struct definitions for pool access.

## Out of scope

- Deciding what to load or when: `stream`.
- Deciding what to drop: `evict`. The pool only provides slots and records which are used.
- Recording requests: `feedback`.
- The CPU tree and voxel encoding: World_Core.

## Relationships

Depends on World_Core constants and Bevy's render device. `stream` writes into it, `evict` frees from it, and `feedback` is read by traversal shaders alongside it. All ray-tracing passes bind it. Shader-side access code lives in `assets/shaders/traversal`.

## Rules

- Sizes are fixed for the process lifetime; there is no reallocation or growth.
- Layout is generated from World_Core constants; no literal sizes in Rust here.
- A slot is addressable by traversal only after its data is fully uploaded; a node must never point to a slot that is not resident.
- Missing children are represented explicitly in the layout so a ray can detect them and emit a request instead of reading garbage.
- Slot bookkeeping is CPU-side and authoritative; the GPU never allocates.
