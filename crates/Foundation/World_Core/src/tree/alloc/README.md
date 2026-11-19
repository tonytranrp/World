# Tree Allocation

## Purpose

This module owns the memory behind the tree: arenas of nodes and leaf bricks addressed by compact integer handles rather than pointers. Stable, index-based storage is what lets the CPU tree be mirrored into a GPU node pool, compacted, serialized and shared across threads.

## Scope

- Arenas for interior nodes and leaf bricks with typed u32 handles.
- Free lists and reuse of freed slots so long edit sessions do not grow memory without bound.
- Copy-on-write storage pages, so snapshots holding an older root stay valid and are freed with their last holder (decision 0004).
- Compaction passes that renumber handles to reduce fragmentation, with an explicit remap result.
- A change journal of allocated, freed and rewritten slots, which the render side uses to upload only what changed.
- Capacity accounting and statistics for budgets and benchmarks.

## Out of scope

- Node field layout, child masks and palettes: the node module.
- Deciding what to write and building new paths: the mutate module.
- The GPU-side pool, LRU eviction and the feedback buffer: World_Render.
- General-purpose allocators for unrelated data.

## Relationships

It depends on the node module for the shape of stored items. The mutate module allocates and frees through it; the tree root module hands out read access built on it; serialization and World_Render consume its journal and remap information.

## Rules

- Handles are opaque and never dereferenced across a compaction without applying the remap.
- A freed slot is never visible to a live reader.
- Allocation is deterministic given the same sequence of operations, so tests can compare structures.
- No unsafe aliasing: any unsafe code is local, documented and covered by property tests.
- Allocation failure and capacity limits are reported, not hidden.
