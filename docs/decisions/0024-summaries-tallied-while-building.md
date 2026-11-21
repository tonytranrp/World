# 0024: Chunk summaries counted from stored bricks, not leaf by leaf

Status: accepted, 2026-11-21

## Context

Decision 0022 moved a replaced chunk's summaries (the representative
material and solid voxel count of every mixed node of levels 1 to 3,
`ChunkSummaries::compute` in World_Render's `brick_cache/stream/summaries.rs`)
from the render thread to World_Game's residency workers, which build the
chunk. The render thread's summary work fell from about 2,850 to under 60 a
frame, but each worker paid what the render thread had paid: in its loaded
walks a generated chunk took 130 to 143 ms of worker time against 114 to 126
before, generation throughput fell 5 to 10% and the full-detail sphere
filled more slowly in two of three runs. Its first next step was to tally
the summaries while the chunk is built instead of walking it again.

Where that walk went: `count_voxels` reached every leaf of every brick
through a `NodeView`, one view per leaf, each reading the brick header
again and ranking its slot, so a surface chunk (about 6,500 bricks and
300,000 non-air leaves) cost 50 to 60 ms of worker time under load. Two
thirds of those leaves are full leaves of their brick's default material,
which since decision 0021 have no words at all: the brick header's default
mask says how many there are. The rest are one word (another full
material), a three-word record (one material and air: the mask's
population is its count) or a record and a palette block (almost all two
materials at one bit: a popcount gives both counts).

## Decision

- **World_Core gives a brick's counts from its storage.** `NodeView` gains
  `brick_counts(each)`: for a level-1 node, pass its solid voxels to `each`
  as (material, count) pairs, a material possibly more than once, and
  return true; the default returns false and passes nothing, so dense
  grids and other sources keep the leaf walk. The world tree's
  `BranchRef::brick_counts` reads the header (default mask and material,
  other-full mask), then the stored leaves in order without ranking:
  one count for all the default leaves, 64 for each other full leaf, the
  mask's population for a record of one material, and the palette's counts
  for a packed leaf (`leaf::count_solid`: a popcount of the index words at
  one bit, an index scan above). World_Core still knows nothing of
  summaries; the trait method is a read of its own layout, like
  `palette_block`.
- **World_Render's `count_voxels` uses it for level-1 nodes**, so both the
  workers (`ChunkSummaries::compute`) and the render thread's fallback
  (`Residency::summary`, `warm_chunk`) count bricks this way. Summing
  per-material weights does not depend on how they are grouped or ordered,
  so every `Tally`, hence every summary, is the one the leaf walk gives.
- **The part is built once.** `ChunkSummaries::compute` collects its
  summaries in a vector (children before their parent, in slot order, as
  before) and builds the table part with `Part::from_grouped`, which
  allocates every array of the part's path tree at its final length.
  Inserting one by one grew each array by a quarter at a time, fifteen
  reallocations on the way to 64 values; after the counting change those
  reallocations were the largest part of what was left (a third of the
  samples in `compute`).
- `VoxelTree::apply` with an empty batch is a no-op even on a tree whose
  root is not resident (`set_voxels` returns before looking at the root).
  Decision 0022 found that `StreamedWorld::apply` before anything has
  loaded, with every edit waiting for its chunk, tripped the debug
  assertion "residency was checked above".

## Alternatives considered

- **Tallying in the builder**, decision 0022's plan. `build_region` sees
  every leaf as it writes it and could count them on the way. It needs a
  generic hook threaded through the builder's chunk, brick and branch
  steps (World_Core cannot know the summary type), and through every path
  that makes a chunk a worker hands over: generation, coarse building,
  decoding saved and warm chunks, and the pending edits applied as a chunk
  joins, which are edits, not builds. The render thread's fallback would
  gain nothing. Reading the stored brick is the same arithmetic on data the
  builder has just written, works for every chunk however it was made, and
  needs one trait method. After this change summaries are under 4% of a
  generated chunk's worker time (below), which the hook could at best
  remove; not worth its plumbing now.
- **Deriving level-2 summaries from node masks and uniform children
  alone.** A level-2 node's uniform children are cheap, but its mixed
  children are bricks whose summaries need their leaves' counts; there is
  no shortcut short of storing counts in the tree, which would cost bytes
  in every brick (decision 0021 just removed bytes) and an update on every
  edit.
- **Caching counts in the brick header.** Same objection: bytes and edit
  cost for a value read once per chunk load.

## Consequences

- **Identical summaries.** The tests decision 0022 added pass unchanged
  (`worker_summaries_equal_the_render_threads_on_real_terrain`,
  `an_edited_chunk_never_takes_summaries_of_its_old_content`,
  `worker_summaries_give_the_gpu_cache_the_same_records`), but they compare
  the workers with the render thread, which now counts the same way. So
  two new tests compare with counting voxel by voxel:
  `brick_counts_match_counting_every_voxel` (World_Core, property test:
  every brick of random edits, palettes up to 40 materials and every index
  width, other full leaves, per material), and
  `stored_brick_counts_give_the_summaries_of_counting_every_voxel`
  (World_Render: the default terrain's surface chunk, the chunk below it
  and the coarse surface chunk, every summary equal to the one computed
  through a view that hides the brick storage). `Part::from_grouped` is
  checked against inserting one by one, with parents before and after
  their children, and refuses entries out of order.
- **Worker time per chunk**, `crates/Engine/World_Render/tests/terrain_summaries.rs`
  (`cargo test --release -p world_render --test terrain_summaries --
  --ignored --nocapture`): the 107 chunks of decisions 0020 and 0021 built
  as the workers build them, columns computed once per footprint as the
  workers' column cache shares them, two rounds, one thread. Release
  builds of 46310cc and of this change, alternating, while other agents'
  builds ran (load averages at the start of each run, CPU speed limit from
  `pmset -g therm`):

  | Run | Load | Limit | Columns + build | Summaries, mean (median) | Share |
  |---|---|---|---|---|---|
  | before | 143 | 60 | 252.6 ms | 57.7 ms (57.1) | 22.8% |
  | after | 68 | 46 | 282.1 ms | 10.9 ms (7.3) | 3.9% |
  | before | 48 | 46 | 271.4 ms | 61.4 ms (48.7) | 22.6% |
  | after | 120 | 75 | 259.3 ms | 9.9 ms (6.0) | 3.8% |
  | before | 75 | 100 | 278.5 ms | 63.2 ms (64.0) | 22.7% |
  | after | 46 | 100 | 267.9 ms | 9.6 ms (5.3) | 3.6% |

  Summaries are 5 to 7 times cheaper in every pair, from 23% of a full-detail
  chunk's worker time to under 4%, and the whole per-chunk cost fell from
  310 to 342 ms to 269 to 293 ms (the build itself, unchanged code, moved
  by up to 30 ms between runs). Coarse chunks (234 mixed nodes) were 2 to
  3% before and after, about 1 ms; this change does not help them much.
  The counting change alone (an earlier pair of runs, columns computed per
  chunk) gave 11.9 to 15.0 ms against 50.2 to 62.1; building the part at
  once then fits the lower figures above, but the two after builds were
  not run side by side and the difference is within the noise.
- **Where the rest goes.** Sampling (`sample`) the counting change before
  the part change: of the samples in `compute`, 36% grew the part's
  arrays, 28% counted bricks, 26% made the child views of levels 2 and 3
  (`BranchRef::child_at`), and the rest was `compute`'s own loop, its
  scratch tree and freeing.
- **Floor walks.** 3,000-frame 8 m/s release walks, `--mac-floor`, saves
  deleted before each run, `--no-default-features` builds of 46310cc and
  this change (not the `dev` builds of decision 0022, so absolute figures
  differ from its table), alternating, frames 101 to 3000; the machine was
  loaded throughout and throttled during the first two pairs:

  | Run | Load | Limit | Mean / p50 / p95 / p99 / max | Over 17.5 / 25 / 34 ms | Generated, worker ms a chunk | Bricks at frame 2000 | Holes, first 3 and last 2 intervals |
  |---|---|---|---|---|---|---|---|
  | before | 35-106 | 100-64 | 20.4 / 17.1 / 37.0 / 50.8 / 85 | 44 / 29 / 7.7% | 906 (15.1/s), 230 ms | 49,500 | 193, 163, 157 ... 77, 72 |
  | after | 106-72 | 64-33 | 20.9 / 17.5 / 36.6 / 54.0 / 115 | 50 / 27 / 7.4% | 909 (15.2/s), 200 ms | 48,500 | 230, 163, 105 ... 110, 108 |
  | before | 72-64 | 33-35 | 20.5 / 18.3 / 34.4 / 42.1 / 67 | 55 / 27 / 5.5% | 823 (13.7/s), 230 ms | 46,800 | 238, 189, 142 ... 141, 128 |
  | after | 64-59 | 35-33 | 20.5 / 18.4 / 34.5 / 41.8 / 60 | 55 / 27 / 5.7% | 957 (15.9/s), 190 ms | 40,200 | 234, 166, 103 ... 94, 105 |
  | before | 26-75 | 100 | 19.2 / 16.9 / 33.6 / 42.3 / 51 | 37 / 23 / 4.7% | 800 (14.5/s), 214 ms | 29,800 | 241, 196, 159 ... 117, 102 |
  | after | 75-95 | 100-80 | 19.0 / 16.9 / 33.1 / 41.8 / 66 | 35 / 21 / 3.9% | 930 (16.9/s), 179 ms | 30,100 | 236, 180, 132 ... 92, 87 |

  - A generated chunk took 30 to 40 ms less worker time (230 to 200 and
    190 ms, 214 to 179 ms) in all three pairs, the size of the summary
    saving the test above measures, and the after runs generated as many
    chunks (one pair) or 16% more (two pairs).
  - Holes in the third interval were lower after in all three pairs (105,
    103 and 132 against 157, 142 and 159); the first two intervals and the
    last ones were not consistently different. Frame times were close within each
    pair and moved neither way consistently (the frame thread no longer
    summarizes these chunks since decision 0022). **Only the worker time
    is shown beyond the noise**; whether
    the sphere fills faster, and decision 0022's question of fewer bricks
    at frame 2000 (40,200 in one after run, as many as before in the
    others), need quieter runs.
- The render thread's own summaries (the chunks it still warms: evicted
  chunks falling back to coarse ones, edited chunks' coarse chunks,
  refused offers, ancestors) use the same counting, so they are cheaper
  too; not measured separately.
- `NodeView` has one more method with a default; a source that stores
  bricks another way can override it. World_Core's own `representative`
  (coarse summaries for region files) still counts voxel by voxel.
- Next, by expected gain:
  - Measure on a quiet machine whether the full-detail sphere fills faster
    and whether fewer bricks are resident at frame 2000.
  - Coarse chunks' summaries cost about 1 ms for 234 nodes, 4 µs a node,
    which points at fixed costs such as the scratch tree `compute` builds
    to view a chunk (not profiled); a view straight onto `ChunkData` would
    remove that for every chunk.
  - `representative` and `extract_coarse` (saving an edited chunk) could
    use `brick_counts` too.
  - Tally in the builder only if summaries ever matter again at under 4%.
