# 0022: A replaced chunk's summaries are computed where the chunk is built

Status: accepted, 2026-11-21

## Context

Decision 0019 timed the brick cache's `run_frame` on the render thread of
the floor machine (2019 MacBook Pro, i7-9750H, Radeon Pro 5300M, Metal,
`--mac-floor`) during a release walk at 8 m/s. In the 44% of frames over
12 ms, applying changes took 6.6 ms, 5.5 ms of it summarizing replaced
chunks: about 2,850 node summaries a frame on average, in bursts of 5,000
and more, at about 1.1 µs each on the brick cache's pool. A replaced chunk
(decision 0012) forgets what its old content cached, and `warm_chunk`
summarizes its level-2 grandchildren and their mixed level-1 children, then
each level-3 child from those, under `max_summaries_per_frame`; the chunk
itself is summarized when it is applied.

Those summaries depend only on the chunk's content, and every chunk the
render tree swaps in was built off the frame: the residency workers
generate, decode or decompress full-detail chunks and build coarse ones
(`jobs.rs`), and the main thread only moves the finished arenas into the
world and render trees in O(1). What a chunk change carried to the
renderer was its position alone (`VoxelWorldFeed::publish_with_chunks`).

## Decision

- **The workers summarize what they build.** `ChunkSummaries::compute`
  (World_Render, Bevy-free, in `brick_cache/stream/summaries.rs`) gives
  every mixed node below a chunk (levels 1 to 3) the summary the render
  thread would give it, through the same level-2 function `warm_chunk`
  uses (`summarize_level2_node`). World_Game's pool calls it for every
  loaded chunk and every coarse chunk it builds, and the result travels
  with the chunk (`Done::Loaded`, `Done::Coarse`, the `ready` queue). World
  depends on World_Render already; World_Render does not learn about
  World_Game.
- **They are built as the cache stores them.** The summary cache is split
  at the chunk level (decision 0014), so a chunk's summaries are exactly one
  part of the table. The worker builds that part (`table::Part`, paths below
  the chunk, no keys) and the render thread inserts it with one hash map
  insertion (`Table::insert_part`), replacing what the old content cached.
- **They carry the content they describe.** A `ChunkSummaries` holds the
  `ChunkData` it was computed from. World_Core gained `ChunkData::
  same_arenas`: two chunk contents are the same arenas when they hold the
  very same page directories and root. Pages are copied on write while
  anything else shares them, so while the summaries hold their `ChunkData`,
  content that compares equal to it has not been written since, through
  whatever trees it moved; content that is merely equal compares unequal.
  `VoxelSource::shared_chunk` (trees and snapshots; `None` for other
  sources) gives a source's chunk to compare with.
- **Checked twice, used once.** When the main thread puts a chunk in the
  render tree (a join, a coarse chunk shown), it keeps the worker's
  summaries for the frame; `take_render_summaries` hands over only those
  that still describe the render tree after the frame's edits, and
  `feed_renderer` offers them with the frame's chunk changes
  (`VoxelWorldFeed::offer_summaries`, carried in `FeedFrame` and merged
  like the rest). The renderer keeps offers only for chunks waiting to be
  applied, and applies a chunk with its offer only if the snapshot it is
  applying from holds exactly that content (`Residency::take_offer`).
  Otherwise, and for every chunk without an offer (an evicted chunk going
  back to its coarse chunk, an edited chunk's new coarse chunk), it warms
  the chunk as before. An edited chunk can therefore never pair with
  summaries of its old content: edits applied as a chunk joins, an edit in
  the same frame, an edit while the chunk waits, or a newer replacement all
  write new pages, and the check fails.
- **A chunk applied with its offer is charged `OFFER_CHARGE` (64)
  summaries** against `max_summaries_per_frame`, besides the summaries it
  still computes (its own, and its ancestors' when their paths are
  rewritten): its insertion is O(1), but it still buries what it held and
  rewrites its path. Every per-frame cost stays bounded by a count
  (decision 0012), and the `grown` and `done` masks are untouched: an
  offered chunk leaves the table exactly as warming it would.

## Alternatives considered

- **Moving the summary function to a Foundation crate.** It needs only
  World_Core's views, but it must stay the very function the renderer's
  fallback uses, and the part it fills is the renderer's table layout;
  World_Game may depend on World_Render, so it stayed there.
- **Pairing summaries with chunks by order alone** (drop an offer when a
  later change touches its chunk). It needs no World_Core change, but
  `FeedFrame::merge` loses the order between a frame's chunk and brick
  changes, and pending edits applied on joining are not changes at all.
  The arena check makes the pairing a property of the content.
- **A content hash** to pair them: it would cost a walk of the chunk on
  the render thread, about what summarizing it costs.
- **Keeping coarse chunks' summaries** for when an evicted chunk falls back
  to its coarse chunk: tens of MiB for the coarse ring, held for a case
  that summarizes a small chunk. Not done.

## Consequences

- **Identical results.** `worker_summaries_equal_the_render_threads_on_real_terrain`
  (World_Render) builds the default terrain's surface chunk and the chunk
  below it at full detail and a neighbouring surface chunk from coarse
  cells, as the workers build them, and checks every summary a worker
  computes against the one the render thread caches for the same node, the
  node counts, and that a table given the offers ends with the same summary
  cache and writes the same records. `an_edited_chunk_never_takes_summaries_of_its_old_content`
  edits one chunk and replaces another with an equal copy after their
  summaries were computed: both are refused and summarized on the render
  thread, with the same records as never offering them. Applying the three
  chunks computed under a twentieth of the summaries with their offers.
  `worker_summaries_give_the_gpu_cache_the_same_records` (World_Game) runs
  the CPU model of the cache over the streamed world for 240 frames of a
  walk that digs every half second and meets edits waiting for unloaded
  chunks: with every change applied at once, the cache fed the workers'
  summaries serializes byte for byte the uploads of the one that computes
  them, every frame, and a third under the default budgets passes `verify`
  at the end.
- Steady frames (101 to 3000) of 3,000-frame 8 m/s walks, release (`cargo
  build --release -p world`), alternating, saves deleted before each run;
  load averages and CPU speed limits (`pmset -g therm`) before and after
  each run. Other agents' builds ran throughout:

  | Run | Load | Speed limit | Mean / p50 / p95 / p99 / max | Over 17.5 / 25 / 34 ms | Records, bricks at frame 1000; 2000 | Holes, first and last 5 s intervals |
  |---|---|---|---|---|---|---|
  | before | 20-41 | 100 | 21.9 / 20.7 / 37.0 / 42.8 / 65 ms | 63 / 34 / 8.0% | 1.15 M, 33,400; 1.27 M, 62,300 | 220, 108, 62 ... 19, 21 |
  | after | 41-98 | 100-80 | 20.2 / 17.4 / 33.5 / 41.0 / 79 ms | 49 / 27 / 4.6% | 1.18 M, 32,500; 1.27 M, 44,800 | 218, 106, 62 ... 27, 30 |
  | before | 98-80 | 80-46 | 20.9 / 18.9 / 34.6 / 43.1 / 195 ms | 58 / 26 / 5.4% | 1.22 M, 30,100; 1.28 M, 62,100 | 214, 93, 44 ... 10, 0 |
  | after | 80-47 | 46-40 | 19.0 / 16.8 / 32.2 / 44.5 / 187 ms | 34 / 14 / 3.3% | 1.10 M, 28,100; 1.46 M, 40,500 | 242, 199, 164 ... 0, 0 |
  | before | 47-40 | 42-46 | 21.5 / 19.3 / 37.3 / 46.1 / 82 ms | 59 / 28 / 8.0% | 1.36 M, 35,300; 1.34 M, 59,600 | 161, 26, 0 ... 63, 54 |
  | after | 40-47 | 46-40 | 21.3 / 17.9 / 35.9 / 55.1 / 150 ms | 52 / 27 / 6.9% | 1.00 M, 28,200; 1.16 M, 54,600 | 227, 139, 81 ... 13, 9 |
  | before | 17-14 | 100 | 18.0 / 16.7 / 27.3 / 39.2 / 114 ms | 28 / 8 / 1.9% | 1.26 M, 30,000; 1.40 M, 38,600 | 228, 114, 21 ... 0, 0 |
  | after | 14-11 | 100 | 17.9 / 16.7 / 28.1 / 42.3 / 98 ms | 22 / 7 / 2.2% | 1.37 M, 42,000; 1.40 M, 42,200 | 142, 10, 0 ... 0, 0 |

  - The render thread computed 56,000 to 119,000 summaries by frame 2000
    (about 30 to 60 a frame: expansions, ancestors rewritten, and chunks
    that fell back), against about 2,850 a frame in decision 0019's walk.
    1,057 to 2,671 chunks were applied with the workers' summaries by
    frame 2000, and no offer was ever refused as stale on these walks (no
    edits).
  - In the three loaded pairs (load 20 to 98, speed limit 40 to 100) the
    median was 1.4 to 3.3 ms lower, p95 1.4 to 3.5 ms lower and the share
    of frames over 17.5 ms 7 to 24 points lower after, with the after runs
    under as much load and throttling as the before runs or more. In the
    fourth pair, on a quieter machine (load 11 to 17, no throttling), the
    two were the same: 18.0 against 17.9 ms mean, 16.7 median both, p95
    27.3 against 28.1. p99 and max did not move consistently in any pair.
    That fits the cause (the warming ran on the brick cache's 8 threads,
    which take long only when other work has the cores), but the load
    changed during the runs by more than the difference, and decision 0019
    saw the unchanged binary's p95 range 28.8 to 39.1 ms. **The frame-time
    gain is not shown beyond the noise.** What is shown is the render
    thread's work: 2,850 summaries a frame down to under 60.
  - **The workers pay for it.** Summarizing a chunk costs a worker about
    what it cost the render thread, so the work moved; it did not shrink.
    In the loaded pairs generated loads took 130 to 143 ms of worker wall
    time a chunk on average against 114 to 126 before (the summaries are
    counted in it), the after runs generated 20.0 to 22.1 chunks a second
    against 22.5 to 23.9, and in two of three the full-detail sphere filled
    more slowly at the start (holes 242, 199, 164 and 227, 139, 81 in the
    first three intervals, against 161 to 220, 26 to 108, 0 to 62 before).
    In the quiet pair the after run filled it faster (142, 10, 0 against
    228, 114, 21; 74 against 77 ms a chunk). On a busy machine the holes
    can rise.
  - Bricks at frame 2000 were fewer in all three loaded after runs (40,500
    to 54,600 against 59,600 to 62,300) but not in the quiet pair (42,200
    against 38,600; 42,000 against 30,000 at frame 1000). With nothing
    waiting, stage 1 can expand into a replaced chunk the frame it lands
    instead of loading bricks elsewhere meanwhile; whether the loaded runs
    lost detail was not measured.
- Writing that test found that `VoxelTree::apply` with an empty batch on a
  tree whose root is not resident (`StreamedWorld::apply` before anything
  has loaded, every edit waiting) trips the debug assertion "residency was
  checked above". Release builds ignore it; it is not fixed here.
- World_Core's tree keeps its encoding; it gained `ChunkData::same_arenas`,
  `TreeSnapshot::chunk_data` and `shared_chunk` on `VoxelSource` and
  `TreeRead` (default `None`).
- Next, in order of expected gain:
  - Summarize where the chunk is built, not after: the builder writes every
    leaf once and could tally it as it goes, so a worker would not walk the
    chunk again, and the holes would return to where they were.
  - Find out why fewer bricks are resident at frame 2000 (stage 1's split
    of the upload budget between expansions and loads when chunks no longer
    wait), and whether the picture lost detail.
  - Offer the coarse chunk's summaries again when an evicted chunk falls
    back to it, if the walks show those fallbacks matter: the render thread
    still summarizes them.
  - Measure on a quiet machine.
