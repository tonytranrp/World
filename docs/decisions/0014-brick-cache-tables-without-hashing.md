# 0014: The brick cache keeps its per-node state in sparse arrays, not hash maps

Status: accepted, 2026-11-20

## Context

The engine plan's aim (the voxelbee reference) is RAM and VRAM that scale
with what is on screen. Decision 0012 split the brick cache's residency
table into small hash maps and estimated them at 113 bytes a slot. Walking
8 m/s in `--mac-floor` on the floor Mac (2019 MacBook Pro, i7-9750H, Radeon
Pro 5300M, development build), the process's heap was measured with
`MallocStackLogging=lite` and `malloc_history -allBySize` (live allocations
by call stack) the moment the cache logged frame 1,000, about 47 s in, with
702,000 residency records:

| What | MiB | Notes |
|---|---|---|
| World tree pages (World_Core `push_page`) | 306 | hot chunks, coarse tier, chunks built and waiting to join |
| Column footprints (World_Gen `columns`) | 256 | the whole `column_cache_mib` cache, 32 footprints of 8 MiB |
| Footprint copies in workers (`Vec<Column>::clone`) | 32 | |
| Brick cache residency table | 180 | 702,000 entries of 112 bytes: 269 bytes each |
| Brick cache summary caches | 50 | 16 bytes of payload per summary |
| Cave grids, upload batch, Bevy render world, startup, task pools and thread stacks, other | 133 | |
| Heap total | 957 | |

Outside the heap the resident set adds about 600 MiB of clean code pages
(the development build's Bevy dylib) and 42 MiB of IOSurfaces (the
swapchain); the GPU pools live in VRAM. The warm tier stayed empty on
these runs: with other work on the machine, the residency workers were
never idle enough to compress evicted chunks.

The two brick cache maps (230 MiB) were the largest item that was not
voxel data or a configured cache, and most of them was overhead: a hash
slot stores the 16-byte key and a control byte beside the value, and maps
that double sit between seven sixteenths and seven eighths full; these sat
about half full.

## Decision

- **Keys below a level are paths, not hashes.** `Table` keeps the keys
  below its split level per ancestor at that level in a small tree laid
  out like the 64-tree: a node holds its children's values contiguously,
  found by the rank of their slot in a 64-bit mask, and the nodes of its
  children that have values below them the same way. No key is stored and
  no slot is empty. Keys at or above the split, and the parts themselves,
  stay in hash maps.
- The arrays are ring buffers that grow by a quarter, not double, and give
  capacity back once half of it is unused. Inserting or removing shifts
  the shorter side, at most 32 siblings, and nothing when children come or
  go in slot order (expanding, collapsing, freeing a buried subtree), so
  decision 0012's bound holds: nothing grows or rehashes many entries at
  once, and a part that empties is dropped.
- **Summaries use the same table**, split at the chunk level, so a
  replaced chunk still forgets its own summaries at once (`remove_below`).
- `Entry` and `Summary` are unchanged; only where they are stored moved.

## Alternatives considered

- **Shrinking `Entry`** (an enum for the group or brick, the child masks
  only on expanded entries). Level-1 entries, the vast majority, use less
  than half of its 112 bytes, so it would save about as much again; it is
  the next step for this table.
- **Packing `NodeKey` into a `u64`.** Saves 8 bytes a slot and keeps the
  half-empty slots.
- **Sorted vectors per part.** No hashing either, but an insertion into a
  part of 4,000 entries moves hundreds of kilobytes.
- **A smaller column cache.** A tuning choice with a measured hit rate
  (decision 0011), not overhead; see Consequences.

## Consequences

- Live allocations at frame 1,000, one run each, back to back:

  | | Before | After |
  |---|---|---|
  | Residency records | 702,184 | 712,388 |
  | Residency table | 180.2 MiB, 269 B a record | 87.7 MiB, 129 B a record |
  | Summary caches | 50.1 MiB | 18.1 MiB |
  | Heap total | 957 MiB | 832 MiB |

  At the 1.3 to 1.45 million records of decision 0012's long walks the
  table alone is about 180 MiB smaller.
- A synthetic test (640,000 values of 112 bytes, 32 per level-2 parent,
  optimized test build) inserted them in 257 against 615 ms, looked each
  up in 90 against 131 ms and removed them in 196 against 180 ms.
- Resident set and frames over 3,000-frame walks at 8 m/s, alternating
  before and after runs, with other sessions' builds and walks on the
  machine and the CPU speed limit mostly 35 to 57 (`pmset -g therm`). The
  load changed how far the cache refined (525,000 to 1.13 million records
  at frame 2,000) and so the RSS, by up to 900 MiB between runs of the
  same binary; the closest matched pair (1.01 and 1.13 million records at
  frames 1,000 and 2,000 in both, speed limit 35 to 37) gave:

  | | Peak RSS | Steady frames: mean / p95 / p99 / max |
  |---|---|---|
  | Before | 1,512 MiB | 20.5 / 35.2 / 44.3 / 56.7 ms |
  | After | 1,278 MiB | 21.8 / 37.7 / 47.1 / 59.4 ms |

  and two pairs on the previous base (96-byte entries, matched within 3%
  in records) 1,529 and 1,527 against 1,397 and 1,398 MiB. Over all runs
  on this base the steady frame means were 20.5 to 34.9 ms before (seven
  runs) and 21.8 to 31.0 ms after (five runs), with no difference
  beyond the load's: a lookup takes one to three rank steps after the
  part's hash instead of a second hash probe, and siblings share cache
  lines. The RSS includes about 600 MiB of clean code pages of the
  development build.
- The table no longer stores keys, so iterating it rebuilds them; `iter`
  collects the entries below the split first (only `verify` and tests
  iterate).
- Next: the column cache (256 MiB, 8 bytes a column where 4 would do; it
  served 0.7 to 2.7 hits per computed footprint on these walks, but only
  0.1 to 1 while the machine was busier and generation fell behind), then
  `Entry` itself.
