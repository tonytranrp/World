# 0012: The brick cache bounds every per-frame cost by counts

Status: accepted, 2026-11-20

## Context

Decision 0010 bounded stage 1's queue pops per frame. Walking at 4 and
8 m/s in `--mac-floor` on the floor Mac (2019 MacBook Pro, i7-9750H,
Radeon Pro 5300M), frames still averaged 20 to 25 ms with a worst frame of
130 to 260 ms. Timing `run_frame` and sampling it found several costs no
budget covered:

- **Expansions** were limited only by upload bytes, and an expansion's
  records are small: one frame expanded 801 nodes in 73 ms, another 2527
  in 88 ms.
- **Changed bricks** cost milliseconds each (145 ms for 20). They were not
  player edits but chunks the residency swapped into the render tree
  (loaded, unloaded, full to coarse and back), reported as one brick each.
  Rewriting that brick's path summarized the whole new chunk (a generated
  surface chunk with caves has about 5,700 mixed nodes: 6.3 ms in release,
  over 10 ms in a development build). Worse, everything the cache held
  below the chunk off that one path kept describing the old content:
  records of coarse cells where full detail now was, never refined because
  their records said "uniform" (a test reproduces it).
- **Growth pushes** were not counted. A pass popped its 16,384 nodes but
  pushed about 230,000 children a frame, mostly level-1 nodes whose bricks
  were resident, only to pop each, look it up (a cache miss) and find
  nothing to do. A pass restarted whenever the eye moved more than
  `regrow_voxels` (16), so with slow frames it restarted every frame and
  never finished.
- Smaller ones, found once the above were fixed: the residency table
  rehashing at each doubling (over 100 ms at a million entries); a node
  pool pressure change running a whole shrink walk in one frame (157 ms);
  a frame collapsing 1,500 nodes; stage 1 decoding and sorting the ray
  request block it ignores; `Residency::stats` scanning every entry for
  the periodic log line.

## Decision

Every per-frame cost is bounded by a count, never by the clock, so frames
and tests stay deterministic.

- **Replaced chunks are their own kind of change.** `VoxelWorldFeed`
  carries the chunks (level-`CHUNK_LEVEL` subtrees) replaced wholesale
  besides the changed bricks, and World_Game reports its swaps there.
  Applying one drops everything the cache held below the chunk and
  summarizes it afresh; stage 1 refines it again.
- **Changes are queued and applied nearest first under
  `max_summaries_per_frame`** (2048 node summaries, each over up to 64
  children). A replaced chunk is summarized a grandchild at a time, across
  frames if needed, and applied once that is done; a chunk replaced again
  meanwhile starts over. Until a change is applied, the records above it
  describe the old content: a swapped chunk shows its old detail a little
  longer, a far edit appears a few frames late (near ones come first).
  Nothing in a waiting chunk is expanded or loaded, and a waiting chunk
  with no cached summary gets a cheap deterministic stand-in, so work on
  shared ancestors never walks it.
- **Summaries below the chunk level are kept per chunk**, so a replaced
  chunk forgets its own at once, and air and uniform children are answered
  without a cache lookup.
- **Applying a change rewrites only the records on its path** in each
  expanded ancestor, the whole group only when the ancestor's mask
  changed.
- **Expansions:** at most `lod.max_expansions_per_frame` (128), and only
  while they have computed fewer than `max_summaries_per_frame` summaries;
  the rest are carried, as upload-limited candidates already were.
- **Growth queues only children that can need work.** Each entry keeps
  the slots of its children that have records and, at level 2, of those
  with resident bricks; a level-2 node queues only children without one.
  **Pushes count** against `lod.max_visits_per_frame` like pops.
- **A pass in progress restarts only after the camera moved
  `lod.restart_voxels`** (128, at least `regrow_voxels`). Smaller moves let
  it finish, since every node is judged from the current view when taken.
- **Shrink** has its own budget, `lod.max_shrink_visits_per_frame` (4096),
  since it only frees memory; it skips children without records and
  level-1 children without bricks, and stops after
  `lod.max_collapses_per_frame` collapses (256), resuming there. A
  pressure change restarts it at the growth budget and holds the pressure
  until that pass is done, instead of walking the whole table at once.
- The residency table reserves what a full node pool typically holds
  (7/16 of its records), so it never rehashes mid-walk; `stats` counts
  node groups and brick ids instead of scanning.

## Alternatives considered

- **Wall-clock budgets.** Simplest to tune, but frames would depend on the
  machine and its load, and tests could not be exact.
- **Reporting every brick of a swapped chunk as changed.** Correct, but a
  chunk has up to 4096 bricks, each rewriting its path.
- **A bucket queue by log2 pixels** instead of the heap. Cheaper pushes,
  but ties within a bucket break the exact best-first order decision 0010
  tests, and few pushes remain once children without work are skipped.
- **Faster leaf counting.** Reordering the palette checks and reusing the
  tally saved 4% on the chunk bench; the walk is bound by visiting every
  leaf of every mixed brick, so it had to be spread instead.

## Consequences

- Walking in `--mac-floor`, development build, three runs each, back to
  back (a one-core Miri run was on the machine throughout), steady frames
  (101 to 1500):

  | Speed | Before: mean / p95 / p99 / max | After: mean / p95 / p99 / max |
  |---|---|---|
  | 4 m/s | 20.3-22.2 / 32-37 / 45-55 / 131-158 ms | 15.6-16.5 / 23-25 / 28-29 / 34-37 ms |
  | 8 m/s | 24.6-24.8 / 40-41 / 52-54 / 241-256 ms | 16.8-16.9 / 24 / 26 / 30 ms |

- Stage 1 also refines further than before: at frame 1000 the cache held
  72,000 to 78,000 expanded nodes and 27,000 to 36,000 bricks, against
  22,000 to 36,000 and 8,000 to 13,000, since growth passes now finish.
  The node pool reaches 82 to 89% (it was 25 to 40%), so node pool
  pressure will act sooner on long walks.
- Holes in the full-detail sphere reach 0 within 15 s at both speeds, as
  before, but start higher: in the first two 5-second intervals 169 to
  175 and 21 to 35 holes on average, against 151 to 167 and 11 to 30,
  likely because faster frames take more CPU time from the generation
  workers.
- At 8 m/s the change queue holds 700 to 1,200 changes (far coarse
  chunks wait longest); at 4 m/s 300 to 800. A larger
  `max_summaries_per_frame` (4096) keeps it near empty for about 4 ms more
  per frame.
- `run_frame` now averages 11 to 13 ms while walking: about 6 ms of stage 1
  visits (cache-missing table lookups), 4 ms of changes, 1.5 ms of
  expansions. Fewer visits trade refinement latency for time, as in 0010;
  a CPU mirror of the pool walked by index remains the option for the
  per-visit cost.
