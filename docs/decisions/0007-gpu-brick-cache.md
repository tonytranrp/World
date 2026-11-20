# 0007: GPU brick cache and GPU tree format

Status: accepted, 2026-11-19

## Context

The plan adopted a ray-guided brick cache (engine plan, "What your answers
changed") and decision 0005 sketched its parts. Research since
(`../research/gpu-voxel-cache.md`) compared GigaVoxels, voxelbee's move-to-front
cache, virtual texturing, Nanite, Aokana, Kiln and the 64-tree guide, and
measured what the three target GPUs need. All three use 128-byte cache lines;
the 2019 MacBook Pro's Radeon Pro 5300M has a 2 MB L2 and Apple GPUs about 8 KB
of L1 per core, so the per-ray working set must be small and contiguous.

## Decision

### The GPU has its own format

The CPU tree keeps its edit-friendly layout (decision 0004 with 0005's inline
leaf records). Bricks and nodes are transcoded on upload into a GPU format:

- **Node, 16 bytes, 16-byte aligned:** the 64-bit occupancy mask, a reference
  word, and an info word with a representative material (u16) and a kind:
  internal (the reference is the index of the first child; children are stored
  contiguously and found by `popcount(mask & below(slot))`, one dependent load
  per level), brick (segment and offset of the brick block), not resident (mask
  and material still drawable), or uniform.
- **Brick block, variable size, 16-byte aligned:** a 16-byte header, then the
  leaf masks (8 bytes each, the data a ray steps through), then one payload word
  per leaf, then the palette and index blocks of decision 0005, read only at the
  final hit.

This withdraws decision 0005's goal of uploading CPU pages without a copy; `Pod`
nodes still make the encoder simple.

### Residency is demand-paged and never stalls

- Data never moves on the GPU except by upload. A ray that uses a brick writes
  the frame number into its stamp (a plain store, at most once per frame,
  optionally staggered). When the pool is nearly full, a pass builds an age
  histogram and chooses victims at least 30 frames old.
- A ray that needs a brick that is not resident draws the finest resident level
  and requests the brick once per frame with `atomicMax` on a per-node stamp and
  an appended request keyed by world brick coordinate. Only 32-bit atomics are
  used.
- The coarsest levels (6 and up) for the whole map stay resident.
- Descent stops where a cell is smaller than the pixel footprint or its child is
  not resident. Full per-voxel bricks are therefore needed only within about 10
  to 20 m; leaf masks and node masks carry detail beyond.
- voxelbee's move-to-front cache is rejected: moving data costs bandwidth,
  atomics and pointer fix-ups, and breaks contiguous children.

### Two stages

1. **Distance-driven:** the CPU chooses nodes and bricks by distance and view,
   with hysteresis. No readback.
2. **Ray-guided:** brick payloads are paged from ray requests read back through a
   ring of buffers, on the same pools.

### Upload

Each frame the CPU encodes changed and requested bricks from the frame's
snapshot into a `StagingBelt`, with edited resident bricks first (at least a
quarter of the budget), allocates space with `offset-allocator`, and records
one apply pass: unlink evicted bricks, scatter the new blocks from the staging
data into the pool, then link them. Budgets: 4 to 8 MiB per frame on the floor
Mac, 16 to 32 MiB on the PC. A destroyed brick that cannot be uploaded this frame
is marked not resident, never left stale.

### Pool sizes and segments

The floor Mac gets a 64 MiB node pool and a 768 MiB to 1 GiB brick pool; the PC
128 MiB and 2 GiB. Segment size comes from the adapter's reported binding limit
at runtime (DX12 and Vulkan clamp to 2^31 - 1 bytes; Metal often allows one
segment), with 256 MiB only as the fallback. This replaces decision 0005's fixed
256 MiB segments.

### Testing

GPU hits are integer records (cell, level, face, material, and whether the level
came from the footprint or from residency). The CPU reference tracer accepts a
forced stop level; exact CPU-vs-GPU comparisons run with a pre-warmed, fully
resident pool and the footprint bias set to full resolution. This extends
decision 0005's integer-hit rule.

## Consequences

- An encoder sits between the CPU tree and the GPU, and must be tested against
  the CPU read interface brick by brick.
- One dependent load per level on the GPU, at the cost of transcoding on upload.
- Coarse occupancy uses the exact "any" rule, which makes porous material look
  fuller from far away; a majority mask (32-byte nodes) can be added later.
- Readback latency of 2 to 3 frames means a fast turn can show coarse detail for
  a few frames.
- Assumptions to verify on the 2019 MacBook Pro: the reported binding limit,
  subgroup size, and that nothing relies on 64-bit atomics.
