# Tree Mutation

## Purpose

This module is the only write path into the tree. It takes a request to change voxels and produces a new, canonical tree plus the exact set of nodes and bricks that changed. Writes use path copying so cost is O(log n) in the tree depth and unrelated subtrees are shared, untouched.

## Scope

- Single-voxel set and clear, box and region fills, and bulk writes of whole bricks.
- Splitting a uniform node when a write lands inside it, and collapsing children back into a single uniform node when they become identical.
- Palette maintenance during writes: adding materials, repacking indices, and shrinking palettes when ids disappear.
- Maintaining child masks and the child array compaction implied by popcount indexing.
- Bulk construction paths, such as building a subtree from a dense array or a sorted voxel stream, used by generators and baking.
- Producing the dirty report: which handles were created, replaced or freed.

## Out of scope

- High-level edit batches, overlays and undo: the edit module, which calls into this one.
- Memory management of slots: alloc.
- Recomputing coarse levels: mip, which runs on the dirty report.
- Destruction logic such as island detection or damage: World_Physics and World_Sim.

## Relationships

It depends on node for layouts and on alloc for storage. The edit module is its primary client; World_Gen writes generated chunks through it. It hands its dirty report to mip and to the render upload path.

## Rules

- After every write the tree is canonical: uniform subtrees are collapsed and equal content has equal structure.
- A write that changes nothing reports nothing.
- Writes never change what an existing snapshot sees: a page a snapshot shares is copied before it is written (decision 0004).
- Behavior is checked against a dense reference array with a large number of random edits, with no divergence allowed.
- Time per edit is bounded by depth, never by world size.
