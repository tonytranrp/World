# Evict

## Purpose

Decides what leaves the GPU pool when it is full. Because the pool size is fixed, every new brick needs a slot, and this folder provides the policy that frees one: least recently used first, informed by what rays actually touched.

## Scope

- LRU bookkeeping over resident bricks and nodes, driven by the touch information gathered from `feedback`.
- Victim selection when `stream` needs slots, with the ability to free a batch at once.
- Protection rules: never evict data needed by the current frame's requests, ancestors of resident children (a child must not outlive a node that leads to it), or coarse mips that guarantee a fallback for far rays.
- Unlinking: producing the updates that clear parent references to evicted entries so traversal sees a missing child and emits a new request instead of reading stale data.
- Eviction statistics for `debug`.

## Out of scope

- Slot storage and the free list: `pool`.
- Deciding what to load: `stream` and `feedback`.
- Deleting voxel data: the CPU tree is never modified here. Eviction removes only the GPU copy.

## Relationships

Depends on `pool` for residency data and on `feedback` for usage. `stream` calls it. `brick_cache` composes it with the other three.

## Rules

- Eviction is by recency of use, not by world position or age.
- The tree stays consistent at all times: no resident node references a non-resident slot unless it is explicitly marked missing.
- Coarse levels are evicted last, so a pixel can always fall back to something.
- The policy is deterministic given a usage sequence so it can be tested and benchmarked without a GPU.
- Eviction never blocks the frame; if enough slots cannot be freed, streaming is deferred.
