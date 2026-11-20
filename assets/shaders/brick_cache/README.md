# Shader Brick Cache

## Purpose

This folder holds the compute shaders that maintain the GPU brick cache itself, as opposed to the shaders that read it. They are the GPU half of World_Render's `brick_cache` module: applying each frame's upload to the node and brick pools, and choosing which bricks to evict when the pool fills.

## Scope

- The apply pass: unlinking evicted bricks, scattering newly uploaded brick blocks from the upload buffer into pool segments, writing brick table entries and linking new records, in that order.
- The eviction pass: an age histogram of the brick table in workgroup memory and the compaction of old bricks into the victim block read back by the CPU.

## Out of scope

- Reading the pools during traversal, stamping bricks as used and emitting requests: `../traversal`.
- Deciding what to upload, budgets and freeing slots: CPU code in World_Render's `brick_cache`.
- The format constants: `../common/format.wesl`, mirrored from World_Render.

## Relationships

- Imports `../common`. Bound and dispatched by World_Render's `brick_cache/stream` (apply) and `brick_cache/evict` (eviction).
- The layouts written here are the ones `../traversal` reads; both follow World_Render's `brick_cache::format`.

## Rules

- Data never moves on the GPU except by upload: these passes never copy a resident block elsewhere.
- Every write is bounds-checked against its buffer, so a bad upload cannot corrupt neighbouring data.
- Only 32-bit atomics; no subgroup operations.
- The CPU decides; these passes only carry out the decisions encoded in their inputs, deterministically apart from the order of compacted victims.
