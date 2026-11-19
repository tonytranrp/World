# islands

## Purpose

This folder answers one question after every edit: did this object fall apart, and into which pieces? It runs connectivity analysis over the edited object and partitions its voxels into disconnected islands. World_Sim reuses the same analysis for structural support.

## Scope

- Connectivity algorithms over voxel data: union-find and flood fill over an object's dense grid or a region of the world tree, with a defined neighborhood (face-connected by default).
- Restricting work to the neighborhood of the edit, so a small chip does not rescan an entire object.
- Incremental and resumable operation: a scan that processes at most N voxels per frame and keeps its state between frames.
- The output: labeled islands with voxel membership, bounding boxes, voxel counts, and center of mass, handed to anchors and debris.
- Splitting a dense grid into independent child grids, preserving each voxel's material and position, and handling the original object's remainder.
- Queuing and coalescing: merging multiple edits to one object in a frame into one scan.

## Out of scope

- Applying damage: damage.
- Deciding if terrain islands are supported: anchors.
- Creating bodies or particles from islands: debris.
- Stress or load-based failure: World_Sim/stress.

## Relationships

Depends on World_Core (grids, tree reads) and the parent destruction budget. Depended on by anchors and debris, and by World_Sim/stress, which builds its support graph on top of these results. Fed by damage.

## Rules

- Island labeling must match a brute-force flood fill over the same grid; that is the reference tests compare to.
- Scans are bounded per frame and never block; an unfinished scan reports as pending, not as a result.
- Labeling is deterministic: the same voxels yield the same islands in the same order.
- It reads voxel data and never writes it, except through the explicit split of a grid.
