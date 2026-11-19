# Stream

## Purpose

The CPU-side producer of GPU data. Given the ranked requests from `feedback` and the edits reported by World_Core, it chooses which nodes and bricks to copy into the pool this frame, packs them, and uploads them without exceeding a fixed bytes-per-frame budget.

## Scope

- Selection: taking requests in priority order until the MB-per-frame budget is spent, and always including edited bricks that are already resident so the picture does not show stale voxels after destruction.
- Reading bricks and nodes from the CPU 64-tree, including generating coarse mips for far rays when a request asks for a lower level of detail.
- Packing into the pool's GPU layout and staging uploads through Bevy's render queue.
- Requesting slots from `pool` and asking `evict` for victims when none are free.
- Budget accounting that adapts to tier and `--mac-floor`, and the metrics reported to `debug`.
- Handling of data not yet generated: asking World_Core and World_Gen's chunk pipeline for content in a way that never blocks the render frame.

## Out of scope

- Detecting what is missing: `feedback`.
- Slot storage and layout: `pool`.
- Replacement policy: `evict`.
- Procedural generation itself: World_Gen. Tree edits: World_Core.

## Relationships

Depends on `feedback` (input), `pool` and `evict` (output side), and World_Core for reads of the tree. Scheduled by `graph` during the first stage of the frame. `brick_cache` coordinates it.

## Rules

- The upload budget is a hard cap per frame; leftover requests carry to later frames.
- Streaming is read-only on the CPU tree and must not hold locks that stall simulation or physics.
- Nothing is made visible to traversal until its upload is complete and its slot is linked.
- Work is time-sliced so a burst of requests (a fast camera turn) cannot cause a frame spike.
- Packing code is pure and deterministic so it can be tested and benchmarked without a GPU.
