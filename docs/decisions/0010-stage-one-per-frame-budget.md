# 0010: Stage 1 works within a per-frame budget

Status: accepted, 2026-11-20

## Context

Decision 0009 split stage 1 (the brick cache's distance-driven selection)
into a growth walk that "runs whole every frame but only descends into nodes
that want refining" and a shrink walk that resumes across frames under
`lod.max_visits_per_frame`. The first runs on the floor Mac (2019 MacBook
Pro, i7-9750H, Radeon Pro 5300M) showed that assumption failing:

- Growth re-ran whole whenever anything changed, including its own
  expansions and every coarse chunk the residency swapped in. A pass visits
  55,000 to 83,000 nodes at about 625 ns each (one cache-missing hash
  lookup per node), so 30 to 55 ms a frame on the floor profile and 0.5 to
  1.3 s frames for most of a minute on the default profile's larger table.
- The shrink walk restarted the moment it finished, so it cost its full
  budget every frame even with a still camera over a settled cache.
- Walking at 4 m/s, nodes taken off a queue counted against the budget only
  if they still existed and were still wanted. A queue gone stale cost ten
  times its budget (the same 16,384 counted visits took 24 ms on some
  frames and 240 ms on others), and a growth pass worked through a queue
  ordered for a view that was gone.

## Decision

- **Growth is a resumable, best-first pass.** It visits at most
  `lod.max_visits_per_frame` nodes per frame from a priority queue ordered
  by the pixels a node's refined cells cover (ties to the lower key, so it
  is deterministic). A pass the budget splits finds work in order of
  importance; run to completion under one view it gives exactly the whole
  walk's answer (tested).
- **Expanded nodes rejoin the pass in progress**, so their children are
  judged by the same pass and expansions no longer schedule a new one.
- **Candidates the upload budget did not reach are carried** to the next
  frame and checked again (still present, still unrefined, still wanted).
- **A pass whose camera moved more than `lod.regrow_voxels` restarts** from
  the root at the current view. Table changes during a pass wait for it to
  finish.
- **Shrink starts a pass by the same rule** as growth: the camera moved more
  than `regrow_voxels`, the pixel angle changed, or the table changed. Stage
  2 keeps looking every frame, because its request holds expire with time.
- **Every node taken off either queue counts** against the budget.

## Alternatives considered

- **Incremental growth from the changed subtrees only.** Correct for edits
  and expansions, but a moving camera changes the answer everywhere at the
  edge of the reach, so a full pass is still needed; a budgeted pass covers
  both cases with one mechanism.
- **A CPU mirror of the node pool, walked by index instead of hash
  lookups.** It would make each visit cheaper (sequential reads) but costs a
  copy of the pool in RAM (64 to 128 MiB) and a second structure to keep
  consistent. Bounding the visits fixed the frame times without it; it stays
  an option if the per-visit cost matters again.
- **Building our crates at opt-level 3 in development.** Measured: no change
  on the walk (the visits are bound by memory latency, not code), and no
  change in incremental rebuild time either, so opt-level 1 stays.

## Consequences

- Stage 1's CPU cost per frame is bounded by `2 * lod.max_visits_per_frame`
  queue pops plus the carried candidates, whatever the camera does. On the
  floor Mac walking at 4 m/s, `run_frame` went from a mean of 86 ms (p95
  353 ms, max 942 ms) to 11.9 ms (p95 23 ms, max 77 ms).
- Under continuous motion the deepest, least important refinement can lag:
  each restart reaches the most pixels first, so detail degrades from the
  far, fine end and catches up when the camera slows.
- A still camera over a settled cache walks nothing, which saves CPU time
  (heat and battery on laptops) even where the GPU bounds the frame.
- `lod.max_visits_per_frame` now sizes both halves; lowering it trades
  refinement latency for frame time.
