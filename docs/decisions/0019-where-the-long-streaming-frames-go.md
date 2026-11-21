# 0019: Where the long streaming frames go, and a node expanded afresh looks nothing up

Status: accepted, 2026-11-21

## Context

Decision 0018 overlapped the brick cache's CPU streaming (`run_frame`)
with the swapchain wait. On the floor machine (2019 MacBook Pro, i7-9750H,
Radeon Pro 5300M, Metal, `--mac-floor`) the release walk's median then sat
on vsync, but p95 was 22 to 24 ms and p99 32 ms, and 562 of 787 frames over
budget had `run_frame` over 10 ms. In frames over 25 ms, applying changes
took 5.3 ms and stage 1's `select_split` 4.2 ms. The question was what
dominates those long frames, and whether bounding or sharing the per-frame
work cuts the tail without losing streaming quality.

## Where the long frames go

Temporary `Instant` probes (not committed) timed each phase of `run_frame`
and counted its work, grouped by the frame's `run_frame` time, over frames
1001 to 2000 of a 3,000-frame 8 m/s walk (release build, `cargo build
--release -p world`, load average 22 to 24, CPU speed limit 100):

| `run_frame` | Share | `run_frame` | `select_split` | applying changes (of which chunk summaries) | expanding | loading | collapsing, evicting |
|---|---|---|---|---|---|---|---|
| up to 8 ms | 33% | 6.1 ms | 1.6 | 0.5 (0.25) | 3.0 | 0.0 | 0.5 |
| 8 to 12 ms | 23% | 10.0 ms | 2.2 | 2.6 (1.8) | 3.2 | 0.5 | 0.8 |
| over 12 ms | 44% | 17.3 ms | 3.3 | 6.6 (5.5) | 3.4 | 2.1 | 1.2 |

| Per frame | up to 8 ms | 8 to 12 ms | over 12 ms |
|---|---|---|---|
| Summaries computed by changes | 236 | 2,218 | 5,143 |
| Chunks applied | 1.1 | 2.3 | 2.1 |
| Nodes expanded (summaries they computed) | 109 (0) | 113 (0) | 120 (0) |
| Bricks loaded | 0.4 | 117 | 425 |
| Nodes collapsed | 68 | 82 | 94 |
| Growth pops and pushes | 16,395 | 14,548 | 13,156 |

- **Applying changes is summarizing replaced chunks.** Nearly all of it is
  `warm_chunk`'s level-2 summaries on the brick cache's pool, about 1.1 µs
  of wall time per summary on 8 tasks, no better than one thread did in
  decision 0012: the generation workers and the other processes on the
  machine had the cores. Planning them on the render thread (counting each
  grandchild's mixed children) took 0.15 to 0.74 ms, rewriting the changed
  paths under 0.1 ms; sorting and queueing changes and `apply_chunk`
  itself under 0.02 ms. The walk asked for about 2,850 summaries a frame on
  average (frames 1001 to 2000), more than the 1,500 to 2,500 decision 0012
  counted, in bursts of 5,000 and more: the long frames are the frames a
  burst of chunks lands in.
- **Expanding is a steady 3 ms, not a burst.** Every frame expands 110 to
  120 nodes, near `max_expansions_per_frame` (128), whatever else it does,
  and collapses 70 to 90 behind: the refined band moving with the camera.
  None of them computes a summary (warming a chunk caches every summary
  below it before the chunk is applied), so their 27 µs each is table work:
  writing a group of up to 64 children looked up all 64 slots for old
  entries, and each mixed child was looked up twice more, inserted, and its
  parent looked up again to set one bit of its mask.
- `select_split` costs about 150 to 190 ns a growth visit. `in_buried`'s
  lookups are not part of it: the nodes being freed numbered under 0.05 on
  average at the end of a frame, so it returns at once.

## Decision

Writing an expanded node's group does only the table work it needs
(`Residency::write_group`):

- Children with entries are exactly the node's `kids` mask (verify checks
  it), so the children that lost their content are found from `kids`
  instead of looking up all 64 slots; a node expanded afresh looks none up.
- When no child has an entry, as for every expansion, no child keeps its
  record or holds a group or brick, so each record is the one
  `group_record` gives a child without an entry, the entries are inserted
  in slot order without being looked up first, and the node's child masks
  are set once for the group instead of once per child.
- `edited_bit` skips hashing while nothing is edited.

The records, entries and masks written are the same as before, so every
count budget of decision 0012 and the residency invariants are unchanged.

A new bench, `cargo bench -p world_render --bench encode --
expand_churn`, collapses and expands again the 34 expanded level-2 and
level-3 nodes of the bench terrain with their summaries cached, the work of
the band a walk refines and coarsens. Three alternating runs of each
binary (load 15): 1.02 to 1.09 ms before, 0.80 ms after, about 25% less
(31 against 23.5 µs a node, collapse and free included).

## Alternatives considered

- **Growth takes what the changes leave of the frame.** Expansions and
  brick loads got the share of `max_summaries_per_frame` the frame's
  changes left, at least a quarter of their budgets, carrying the rest (it
  was implemented with a test that the carried growth settles exactly where
  a frame without changes does). In the same probed binary, alternating
  with an environment switch, steady frames (101 to 3000):

  | Run | Load | CPU speed limit | Mean / p50 / p95 / p99 / max | Records at frame 1000 / 2000 |
  |---|---|---|---|---|
  | without | 22-24 | 100 | 22.3 / 20.7 / 37.4 / 51.7 / 118 ms | 1.00 / 1.24 M |
  | sharing | 16-24 | 100 | 18.8 / 17.6 / 29.0 / 33.8 / 41 ms | 1.02 / 1.37 M |
  | without | 26-16 | 100 | 21.7 / 18.6 / 37.5 / 49.1 / 99 ms | 1.35 / 1.32 M |
  | sharing | 16-22 | 100 | 21.1 / 19.5 / 33.1 / 45.5 / 135 ms | 0.96 / 1.17 M |
  | without | 10-13 | 100-37 | 22.7 / 19.7 / 41.6 / 57.1 / 75 ms | 1.34 / 1.46 M |
  | sharing | 35-39 | 100 | 34.2 / 31.2 / 61.0 / 92.6 / 386 ms | 0.67 / 0.68 M |
  | without | 51-41 | 48 | 21.8 / 20.4 / 36.5 / 43.7 / 141 ms | 1.11 / 1.24 M |
  | sharing | 41 | 44 | 23.1 / 22.3 / 36.7 / 47.8 / 133 ms | 0.92 / 0.97 M |

  and the unprobed build with it against the unprobed base, alternating
  (CPU speed limit 100 for the first pair, 40 to 46 after):

  | Run | Load | Mean / p50 / p95 / p99 / max | Records at frame 1000 / 2000 |
  |---|---|---|---|
  | base | 11-16 | 22.1 / 19.6 / 39.1 / 54.8 / 222 ms | 1.28 / 1.29 M |
  | sharing | 15-14 | 23.1 / 21.3 / 39.0 / 48.8 / 176 ms | 0.89 / 1.14 M |
  | base | 14-10 | 18.9 / 17.3 / 28.8 / 35.3 / 107 ms | 1.35 / 1.35 M |
  | sharing | 10-24 | 24.1 / 22.3 / 40.7 / 56.3 / 94 ms | 0.94 / 0.96 M |
  | base | 24-17 | 23.8 / 22.9 / 38.4 / 46.5 / 73 ms | 1.24 / 1.27 M |
  | sharing | 16-14 | 22.3 / 20.9 / 34.5 / 44.9 / 141 ms | 0.82 / 1.15 M |

  The first two pairs looked like a tail cut (p95 4 to 8 ms lower); the
  other five show none or the opposite, while the cache held fewer records
  at frame 1000 in six of the seven sharing runs (0.67 to 0.96 million,
  against 1.00 to 1.35 in all seven runs without): it refines more slowly,
  as it must, since a burst frame expands a quarter as much. The machine's load (10 to 67, other builds)
  and thermal limit (down to 37%) hide any difference of a few
  milliseconds, so the cost is measured and the gain is not. Not adopted.
- **`max_summaries_per_frame` 4096** (from 8192), to cut the warming
  bursts directly: three runs gave p95 34.0, 43.2 and 40.3 ms against 36.5
  to 41.6 for the runs around them, and pending changes at frames 1000 and
  2000 of 0 to 8 (0 to 1 at 8192). Within the noise; with the sharing rule
  as well, the cache held 0.52 to 0.66 million records at frame 1000. Not
  adopted.
- **Cheaper `in_buried`** (checking only levels that hold a burying node):
  the set it checks was almost always empty, so it would save nothing.

## Consequences

- Steady frames (101 to 3000) of 3,000-frame 8 m/s walks, release,
  alternating, unprobed; load averages before and after each run, CPU
  speed limit 100 except where noted:

  | Run | Load | Mean / p50 / p95 / p99 / max | Over 17.5 / 25 / 34 ms | Records, bricks at frame 1000; 2000 | Holes, first and last 5 s intervals |
  |---|---|---|---|---|---|
  | before | 27-32 | 22.2 / 21.3 / 36.1 / 42.5 / 96 ms | 66 / 35 / 7.2% | 1.19 M, 31,500; 1.24 M, 60,200 | 215, 104, 56 ... 23, 26 |
  | after | 32-42 | 21.5 / 20.1 / 35.5 / 42.6 / 72 ms | 63 / 31 / 6.3% | 1.21 M, 30,100; 1.35 M, 53,800 | 208, 87, 43 ... 13, 13 |
  | before | 42-45 | 22.1 / 21.0 / 36.0 / 42.6 / 66 ms | 67 / 33 / 7.7% | 1.21 M, 30,500; 1.31 M, 56,200 | 209, 86, 41 ... 22, 22 |
  | after (limit 57 at the end) | 45-55 | 21.3 / 20.0 / 34.4 / 40.8 / 105 ms | 63 / 30 / 5.6% | 1.23 M, 33,600; 1.33 M, 55,500 | 205, 87, 44 ... 7, 6 |

  The mean is 0.7 to 0.8 ms lower and p95 0.6 to 1.6 ms lower in both
  pairs, with the after runs under the higher load, about what 110
  expansions a frame at 7.5 µs less each predict; but runs of one binary
  differed by more than that today (the base's p95 ranged from 28.8 to
  39.1 ms over six runs), so the frame-time gain is not shown beyond
  the noise, only the bench's. Streaming quality is unchanged: as many
  records and bricks at frames 1000 and 2000, and no more holes.
- None of the walks today came near decision 0018's numbers (p95 22 to
  24 ms): the machine ran other builds throughout (load 10 to 67, against
  5 to 9 then) and throttled to 37 to 57% of its speed for long stretches.

- The tail is set by the summaries of replaced chunks: bursts of 4,000 to
  8,000 a frame cost 5 to 10 ms on a loaded machine, and every count
  budget in the frame adds to them rather than replacing them. Lowering
  budgets trades refinement for a tail gain this machine cannot measure.
- Next, in order of expected gain:
  - Compute a chunk's summaries where it is built: the residency's
    generation workers already hold the chunk off the frame, and the
    summaries are a pure function of its content, so the feed could carry
    them with the replaced chunk and `warm_chunk` would only insert them.
    That takes the whole 2,850 summaries a frame off the render thread.
  - Failing that, warm a chunk on the pool across frames and join it a
    fixed number of frames later (blocking only if it is not done), so the
    work overlaps whole frames and stays deterministic.
  - Split `summarize_level2`'s units by their planned cost instead of by
    count, so the slowest task is not the one with the surface band.
  - Measure on a quiet machine: with load under 10 and the CPU at 100%,
    decision 0018's runs differed by 2 ms of mean between runs of one
    binary; these differed by up to 15.
