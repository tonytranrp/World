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
  `max_summaries_per_frame`** (8192 node summaries, each over up to 64
  children; 2048 at first, see below). A changed brick is charged the cached summaries on its path
  it forgets (at least one), since the path is summarized again when it is
  rewritten. A replaced chunk is summarized a grandchild at a time, across
  frames if needed, and applied once that is done; a chunk replaced again
  meanwhile starts over, and an edit inside it only marks its path edited,
  since applying the chunk rebuilds everything below it. Until a change is
  applied, the records above it describe the old content: a swapped chunk
  shows its old detail a little longer, a far edit appears a few frames
  late (near ones come first). Nothing in a waiting chunk is expanded or
  loaded, and a waiting chunk with no cached summary gets a cheap
  deterministic stand-in, so work on shared ancestors never walks it.
- **A record only points at a group or brick block laid out for its
  mask.** The GPU finds children and leaves by their rank in the record's
  mask, and a change elsewhere can rewrite the record of a node whose own
  change has not been applied (a waiting chunk, or a node above a deferred
  edit, whose mask in the source changed with it). Each entry keeps the
  mask its group or block was laid out for. When its parent's group is
  written again and that mask differs, or the node lies in a waiting
  chunk, it keeps its record (moved to its new index): it describes the
  old content until the change below rewrites its path. Dropping its
  subtree instead was tried and regressed walking badly: a chunk waiting
  below a level-5 node changes that node's mask, so every rewrite above
  dropped the whole node (13 releases freed 634,000 records in 100
  frames), stage 1 rebuilt it, the change queue grew to 1,200 to 2,300,
  and frames went from 16-17 ms (max 33-38) to 20-21 ms (max 86-99).
  Releasing a subtree walks the entries below, not the record's mask.
  verify checks only the structure of records on a pending change's path
  or in a waiting chunk.
- **Summaries below the chunk level are kept per chunk**, so a replaced
  chunk forgets its own at once, and air and uniform children are answered
  without a cache lookup.
- **Applying a change rewrites only the records on its path** in each
  expanded ancestor, the whole group only when the ancestor's mask
  changed.
- **Expansions:** at most `lod.max_expansions_per_frame` (128), and only
  while they have computed fewer than
  `lod.max_expansion_summaries_per_frame` summaries (2048; they shared
  `max_summaries_per_frame` until that rose to 8192);
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
- **Collapsed and replaced subtrees are freed under
  `max_frees_per_frame`** (16,384 records). A collapse or an applied
  chunk only takes the node's children out of the table into a
  graveyard; their groups, bricks and descendants are freed oldest first,
  that many entries freed or taken out of the table a frame (at most 64
  more), and nothing below the node is refined, loaded, collapsed or
  evicted until they are; freeing the last of them starts a new growth
  pass, since stage 1 skipped that subtree. A collapse counted against
  `max_collapses_per_frame` could
  still free a whole refined region: on a long 8 m/s walk one frame
  applied five chunks that held 236,000 records (58 ms) during a pressure
  change. An edit below such a node waits until its old entries are
  freed.
- **The residency table is split into small maps**: levels 1 and 2 by
  their level-3 ancestor (at most 4,160 entries each), the rest in a top
  map, and a small map that empties is dropped. A single table first
  rehashed a million entries mid-walk (over 100 ms); reserving it up front
  (7/16 of the node pool's records) then failed on long walks: the
  collapses of a node pool pressure change left about 450,000 tombstones,
  and the next insertion that found no room resized 1.38 million entries
  to 3.67 million buckets in one go (282 ms; 296 and 310 ms frames at
  frames 3145 and 3201 of two 6,000-frame 8 m/s walks). Growing or
  rehashing one small map costs at most its own few thousand entries.
  `stats` counts node groups and brick ids instead of scanning.

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
- **A replaced chunk's grandchildren are summarized in up to 8 tasks**
  on Bevy's compute task pool (`summary_threads`, 0 for one per pool
  thread; no threads are spawned, and a panic in a task is raised again),
  only for level-4 chunks, whose grandchildren are level-2 nodes,
  as many as the budget allows, counted exactly before any is computed
  (one summary for each grandchild and one for each of its mixed
  children); the summaries are a pure function of the snapshot, so the
  threads change nothing but the time. With 2048 summaries a frame, one
  thread, the queue never caught up on long 8 m/s walks: counted per 100
  frames, the warming used the whole budget every frame (about 205,700)
  while chunks arrived at 2 to 3 a frame, about 1,500 to 2,500 summaries
  a frame, and pending reached 2,400 to 4,000 by frame 4,000. At 8192 on
  8 threads it stays at 0 to 6 through 6,000 frames, and the frame mean
  went down (17.3 to 15.3-15.5 ms on the long walks, about 1 ms lower in
  1,500-frame walks at 4 and 8 m/s), since the same work takes less wall
  time; p95 and p99 rose by up to 1 to 2 ms. The threads compete with the
  residency's generation workers while the world fills: holes in the
  first 15 s run 5 to 30% higher (185 to 190 and 42 to 49 in the first
  two 5-second intervals at 8 m/s, against 157 to 175 and 30 to 39) and
  the full-detail sphere settles about 2 s later, then holes stay at 0.
  Warming on half the threads cost more frame time (p95 and p99 2 to 5 ms
  higher) without fewer holes.
- `run_frame` now averages 11 to 13 ms while walking: about 6 ms of stage 1
  visits (cache-missing table lookups), 4 ms of changes, 1.5 ms of
  expansions. Fewer visits trade refinement latency for time, as in 0010;
  a CPU mirror of the pool walked by index remains the option for the
  per-visit cost.
- The split table reserves nothing up front: it grows with the entries
  (113 bytes a slot: a 16-byte key, a 96-byte entry, a control byte),
  about 160 to 230 MB at the floor walks' 1.3 to 1.45 million entries,
  instead of a fixed 237 MB (floor) or 474 MB (PC) reservation that
  Windows would count as committed. Lookups of levels 1 and 2 hash twice,
  once into a map of small maps that stays in cache.
