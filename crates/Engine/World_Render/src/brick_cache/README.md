# Brick Cache

## Purpose

The ray-guided brick cache is what lets a world far larger than GPU memory be rendered inside a fixed budget. The CPU 64-tree in World_Core is authoritative; this folder maintains a bounded GPU mirror holding only the nodes and bricks that rays recently needed, at the detail level each pixel needs. GPU memory therefore scales with screen resolution, not world size, which is what keeps the 1 GB pool of the Mac floor viable.

## Scope

- The overall cache lifecycle each frame: read last frame's feedback, decide what to stream, upload within budget, evict, and publish updated pool metadata to the traversal shaders.
- Coordination of the four sub-concerns: `pool` (fixed GPU node pool and brick pool), `feedback` (the buffer where rays record missing bricks), `stream` (CPU-side selection and upload under an MB-per-frame budget), and `evict` (LRU reclamation).
- Coarse mip levels: far rays stop at coarse nodes, so the cache holds and requests mips as well as full-resolution bricks.
- Applying edits: when World_Core reports changed nodes (destruction edits constantly), the corresponding resident bricks are refreshed or invalidated.
- Pool sizing from tier and settings, including the `--mac-floor` cap.

## Out of scope

- Tree structure, edits and generation of voxel data: World_Core and World_Gen. The cache asks for bricks and does not create voxel content.
- Using the pool: traversal shader code belongs to `passes/primary` and the shared WESL traversal modules.
- Screen-sized targets: `gbuffer`.
- Dynamic-object grids, which are dense per-object data handled by `passes/dynamic_box`.

## Relationships

Depends on World_Core for tree reads and edit deltas, on `tier` for pool limits, and on `graph` for scheduling. `passes/primary`, shadow, AO, GI and reflection traversal all depend on it because every ray tracing pass walks the same pool. `debug` visualizes its state.

## Rules

- Pool sizes are fixed at startup and never grow; exhaustion causes eviction or coarser mips, never allocation.
- A missing brick must degrade gracefully (coarser data this frame, requested for the next), never stall the frame.
- Upload bytes per frame are bounded by the budget, and requests are prioritized by screen coverage and proximity.
- The CPU tree is read-only to this folder.
- Pool layout is defined from World_Core constants (brick dimension, tree arity) so shaders and Rust agree.
