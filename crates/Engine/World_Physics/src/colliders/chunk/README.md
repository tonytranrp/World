# chunk

## Purpose

This folder builds and maintains the colliders for the static world, which is a sparse 64-tree far too large to collide with as a whole. It slices the world into collision chunks, keeps colliders only where they are needed, and rebuilds them when voxels change.

## Scope

- Chunking: the world divided into chunks of about 4 m, each represented as a block of 1/16 m collision voxels.
- Interest management: deciding which chunks are active, driven by proximity to awake bodies and to the player, with hysteresis so chunks do not thrash at the boundary.
- Extraction: reading a chunk's region from the CPU tree and downsampling it to collision occupancy.
- Dirty tracking: edits from World_Core's edit API (player, destruction, simulation) mark overlapping chunks dirty, and rebuilds are queued.
- Time-sliced rebuild scheduling and the cache of live chunk colliders, including removal when interest lapses.
- Handling chunk boundaries so adjacent chunks produce no seams or ghost edges.

## Out of scope

- Colliders for moving objects: dynamic.
- Rapier shape construction: rapier.
- Deciding why voxels changed: destruction, World_Sim, or gameplay.
- Terrain generation: World_Gen.

## Relationships

Depends on the parent colliders folder (grid definitions), rapier, and World_Core. Depended on by character (ground and wall queries) and indirectly by every awake body. Sibling of dynamic, with no dependency between them.

## Rules

- A chunk collider is a pure function of the tree contents in its region; rebuilding must produce the same collider as building from scratch.
- Rebuild cost per frame is bounded by the time-slice budget; a backlog delays accuracy, never frame time.
- Chunks with no awake body or player nearby carry no collider.
- Anchoring to bedrock is not decided here; that is destruction.
