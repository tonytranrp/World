# 0026: The beam proves sky tiles, looks one level into occupied cells, and shares its dispatch with the bottom rows

Status: accepted, 2026-11-21

## Context

Decision 0023 added a beam prepass: one thread per 8x8 tile of camera rays
marches a cube that bounds the tile's rays through the tree and the trace
restarts each ray where the march stopped. On the floor machine (2019
MacBook Pro, i7-9750H, Radeon Pro 5300M, Metal, `--mac-floor`, 1280x720
internal) the prepass and the trace together took about 6 to 7 ms of GPU
time where the trace alone had taken about 9. It named three next steps, in
the order they looked most promising:

1. **Sky tiles.** The march ended where its cube touched the tree's edge, so
   sky rays still took 2.2 to 3.9 trace steps each (about half the rays on
   the floor walk).
2. **A tighter stop.** The march ends at the first occupied cell of the
   level `L` it walks at (cells at least as wide as the cube), which can be
   up to one such cell short of a surface. On the grazing floor that is
   most of the trace's remaining steps.
3. **Overlap.** The prepass is 14,400 threads at 720p, a latency-bound pass
   that leaves most of the GPU idle.

The constraint is decision 0023's: hits, requests and brick stamps must be
bit for bit those of the trace from the camera.

## Decision

### Sky tiles (`traversal/beam.wesl`)

- **Outside the tree is empty.** Each step clips the grown cube to the tree
  before choosing its level and cells; on an axis where the cube crosses a
  face of the tree, the free box `V` reaches far past that face
  (`BEAM_BEYOND`, 1e20, finite). This needs every ray to start inside the
  tree, so that a ray found outside has left it for good and the trace ends
  it there. The march therefore does not run when the eye is within `k0 +
  1` voxels of a face or outside (it returns 0, as before for an eye
  outside).
- **Sky.** At the start of a step, if the whole cube lies more than a voxel
  past a face of the tree and that face of the cube moves outward (`c >=
  k1` on that axis, or `c <= -k1` on the low side), every ray of the tile
  has left the tree, and every voxel each passed before lay in some step's
  `V`. The march returns `BEAM_SKY` (-1). The cap on each advance from the
  footprint still holds for every step, so the argument of decision 0023
  applies to every distance the march reached: a ray restarted there would
  start outside the tree and miss, so the ray from the camera misses too.
- **The trace** writes a miss record for a negative start distance without
  traversing: the record the trace writes for any miss, with 0 steps.

### One level into occupied cells (`traversal/beam.wesl`)

When a cell the cube overlaps is a set bit of an internal node of level `L
>= 2`, the beam loads that bit's child node (one more dependent load) and
tests the box of its children (level `L - 1`) that the grown cube overlaps
against its mask, as the trace's whole-node skip tests a box. If none is
set, the box, grown one child at a time along the bundle's motion on each
axis while it stays clear, stands in for the cell's empty cell, at level `e
= L - 1`; otherwise the march ends as before. A uniform child, a brick or
not-resident node with the bit set, or `L < 2` still ends it.

Why this keeps the hit:

- The per-pixel trace descends into the set bit (`L >= 2`, so it is not the
  `level <= 1` stop), and the footprint test cannot fire at level `L` or
  above: the cap on the advance becomes `4^L / footprint` for these cells.
  In the child, a clear bit is an empty cell for every node kind, before
  any brick, request or stamp logic.
- On an axis the cube straddles, the box reaches the shared cell boundary
  (the cube overlaps the last child there), so the corner regions of `V`
  still lie in their own cells' free boxes.

### The bottom rows in the prepass's dispatch (`passes/primary/beam.wesl`)

The prepass dispatch is now one-dimensional: its first workgroups march 8x8
beams over the tile rows above a head band, and each later workgroup traces
one 8x8 tile of the bottom `head` tile rows from the camera, with the
trace's own code (`trace_pixel`). The trace skips those rows in camera mode.
The head trace's throughput-bound rays run while the beams' latency-bound
threads would leave the GPU idle. The head is the bottom of the image
because its rays end near the camera, where a beam saves the least.

`head` is `floor(rows * lod.beam_head_share)` (at most every row but one),
`brick_cache.ron`, 0.125: 11 of 90 tile rows at 720p. It travels in a new
`TraceParams.head` (x), and `VoxelViewTargets.head` sizes the dispatch.
Rays read from the hit buffer (tests) ignore it.

## Tests

`tests/gpu_beam.rs` keeps decision 0023's comparison (every hit record bit
for bit the same with and without the prepass apart from the step count,
the same requested nodes and stamped bricks) and adds:

- three views: level, half a voxel above a ridge's top (the horizon across
  the middle of the image, tiles half sky); out through the tree's +x face
  over a floor running up to it, past 300 lone voxels and a board on the
  tree's last layer of voxels; up through its top face at a plate and lone
  voxels on its top layer.
- a check that every tile marked sky misses on every pixel without the
  beam, and that sky tiles are found in at least three views (396 sky tiles
  of 7,360 at footprint bias 1);
- a run at footprint bias 16 (the config's maximum);
- a run where the prepass traces the bottom quarter of the tile rows (5 of
  23) with every third brick evicted.

Breaking each part makes them fail, checked once each: calling a tile sky
once its cube only touches a face of the tree (view 7, a voxel on the top
layer); testing only one child of the occupied cell's box (view 0); and
giving the box's cells level `L + 1` instead of `L - 1`, which loosens the
footprint cap (bias 16, view 0; bias 1 and 6 do not catch it, which is why
the bias 16 run was added). `gpu_trace`'s CPU comparisons and the 24 golden
images pass unchanged.

On the test scene, steps fall from 5.77 million (decision 0023's beam) to
4.71 million at full resolution, against 7.64 million without a beam.

## Measured

### Bench (`cargo bench -p world_render --bench gpu_timing`)

Real GPU, wall time per submission (no timestamps on Metal), medians of 20
runs. "Both" is the prepass and the trace in one submission, as the game
records them. The bench now has two scenes: `hills` is decision 0023's
(looking down at 8 m of hills over nothing), and `horizon` puts the same
hills on a cell-aligned ground 64 m below them that runs to the tree's edge,
seen almost level, so about half the image is sky leaving the tree kilometres
away. The beam's traversal file was swapped between runs in alternation
(base: decision 0023's `traversal/beam.wesl`; sky: sky tiles only; new: sky
tiles and the look into occupied cells), all with this change's trace and
dispatch, at load averages 12 to 34 (two rounds; ranges are over both
rounds and both footprint settings, full resolution and bias 1).

| Scene | Variant | Steps, head rows | Beam alone | Trace alone | Both |
|---|---|---|---|---|---|
| hills | none | - | - | 7.45-7.76 | 7.55-7.99 |
| hills | base | 16, 0 | 0.69-0.82 | 5.74-5.84 | 6.24-6.38 |
| hills | base | 64, 0 | 0.89-0.98 | 5.51-5.61 | 6.18-6.33 |
| hills | base | 64, 11 | 1.47-1.61 | 5.04-5.47 | 6.29-7.04 |
| hills | base | 256, 11 | 1.45-1.54 | 5.01-5.11 | 6.26-6.33 |
| hills | sky | 16, 0 | 0.72-0.85 | 5.72-5.84 | 6.23-6.40 |
| hills | sky | 64, 0 | 0.94-0.99 | 5.49-5.55 | 6.22-6.26 |
| hills | sky | 64, 11 | 1.50-1.53 | 4.96-5.02 | 6.24-6.29 |
| hills | sky | 256, 11 | 1.46-1.52 | 4.95-5.00 | 6.21-6.28 |
| hills | new | 16, 0 | 0.85-0.94 | 5.33-5.43 | 6.01-6.11 |
| hills | new | 64, 0 | 1.31-1.42 | 4.88-4.99 | 6.01-6.12 |
| hills | new | 64, 11 | 1.59-1.65 | 4.44-4.50 | 5.78-5.87 |
| hills | new | 256, 11 | 1.59-1.63 | 4.43-4.47 | 5.79-5.86 |
| horizon | none | - | - | 7.06-8.01 | 7.14-8.75 |
| horizon | base | 16, 0 | 0.75-0.81 | 5.64-5.72 | 6.18-6.26 |
| horizon | base | 64, 0 | 1.07-1.15 | 5.25-5.32 | 6.11-6.19 |
| horizon | base | 64, 11 | 2.04-2.11 | 4.23-4.33 | 6.07-6.19 |
| horizon | base | 256, 11 | 2.09-2.22 | 4.19-4.32 | 6.05-6.20 |
| horizon | sky | 16, 0 | 0.71-1.07 | 5.56-5.83 | 6.08-6.37 |
| horizon | sky | 64, 0 | 1.08-1.39 | 5.20-5.45 | 6.04-6.34 |
| horizon | sky | 64, 11 | 2.11-2.37 | 4.18-4.46 | 6.07-6.41 |
| horizon | sky | 256, 11 | 2.12-2.59 | 4.11-4.56 | 6.01-6.47 |
| horizon | new | 16, 0 | 0.92-0.95 | 5.22-5.26 | 5.92-6.00 |
| horizon | new | 64, 0 | 1.35-1.45 | 4.63-4.73 | 5.76-5.89 |
| horizon | new | 64, 11 | 2.14-2.26 | 3.65-3.73 | 5.58-5.68 |
| horizon | new | 256, 11 | 2.94-3.04 | 3.53-3.62 | 6.25-6.37 |

(The "beam alone" column includes the head rows' trace where there are
any.)

- **The look into occupied cells** (new against sky, 64 steps, no head)
  cuts the trace by 0.5 to 0.8 ms (hills 5.5 to 4.9 ms, horizon 5.2-5.45
  to 4.7) and costs the prepass up to 0.45 ms more: 0.1 to 0.45 ms net on
  its own.
- **Sharing the dispatch with the bottom rows** pays only with that longer
  march: another 0.2 to 0.25 ms in both scenes (new, 64 steps, 11 rows
  against none), and nothing measurable with the shorter marches of base
  or sky, whose beams finish too soon to hide much. An earlier sweep on
  `hills` (three rounds, new) put 11 rows (1/8) best: both 6.03 to 6.21 ms,
  against 6.10 to 6.24 for 5 rows, 6.16 to 6.33 for 16 and 6.29 to 6.35 for
  none (one outlier at 6.86); in another, 22 rows (1/4) was worse than none
  (6.63 to 6.65 against 6.55 to 6.57).
- **Sky tiles** change nothing measurable at 64 steps on either scene (sky
  against base). Most sky tiles near the horizon need more steps than that
  to reach the tree's edge: the cells around a cube that skims the terrain
  are not much larger than the cube, so each step advances by about its
  width. A larger cap does not pay either: at 256 steps the new prepass's
  longest tiles (grazing ones that now march on into occupied cells, more
  than sky ones: sky at 256 costs little more than at 64) cost more than
  the trace saves (both 6.25 to 6.37 ms on `horizon`). Sky tiles are kept
  at 64 steps because their test costs a few comparisons per step and
  views with high sky do pay (floor walk below).
- **Net (new, 64 steps, 11 rows, the defaults):** both 5.78 to 5.87 ms
  against 6.18 to 6.33 for decision 0023's prepass on `hills`, 5.58 to 5.68
  against 6.11 to 6.19 on `horizon`: 0.4 to 0.5 ms, about 7% less GPU time
  for prepass and trace, and about 25% less than the trace without a beam.

### Floor walk

`world --mac-floor --walk 8 --exit-after-frames 3000`, release builds of
the base commit (`base`) and this change (`new`) with their own assets,
alternating. The machine was the noisiest yet: other agents' builds kept the
load average at 30 to 128 and the CPU speed limit (`pmset -g therm`) at 31 to 57%, mostly 33 to 37%,
so frames were CPU-bound (median 17 to 22 ms either way).

Sampled (`sample_every_frames: 50`; the `gpu passes` and `primary rays`
lines at frames 1000 and 2000; ms mean / min). The new `beam` pass includes
the trace of the bottom 11 tile rows, which the new `trace` pass no longer
does.

| Run | Load | CPU limit | Beam (1000; 2000) | Trace (1000; 2000) | Steps a ray (sky rays) |
|---|---|---|---|---|---|
| base | 6-94 | 100-35 | 2.8 / 1.9; 2.8 / 1.8 | 9.8 / 5.3; 11.0 / 6.5 | 9.9 (4.0); 9.8 (3.4) |
| new | 94-98 | 35 | 3.2 / 2.3; 4.4 / 2.2 | 6.6 / 4.7; 9.5 / 5.2 | 8.3 (2.4); 8.0 (1.3) |
| base | 98-107 | 35-37 | 3.4 / 1.9; 3.0 / 1.3 | 10.9 / 5.9; 10.3 / 5.8 | 9.0 (3.0); 9.1 (2.4) |
| new | 107-97 | 37-35 | 4.1 / 2.2; 4.6 / 2.4 | 8.0 / 4.2; 9.5 / 4.3 | 8.1 (2.2); 8.1 (1.6) |
| base | 97-89 | 35 | 2.8 / 1.6; 3.0 / 1.6 | 8.8 / 5.2; 10.7 / 5.8 | 8.9 (2.9); 8.9 (2.3) |
| new | 87-122 | 35 | 4.6 / 2.4; 4.2 / 2.3 | 9.2 / 4.7; 8.7 / 4.5 | 7.8 (1.9); 8.1 (1.4) |
| new | 77-108 | 33-37 | 3.9 / 2.4; 4.0 / 2.3 | 8.3 / 4.6; 8.7 / 4.7 | 8.1 (2.2); 8.1 (1.6) |
| base | 108-127 | 37-35 | 3.1 / 1.6; 3.0 / 1.6 | 9.9 / 5.0; 11.7 / 5.6 | 9.0 (3.1); 9.2 (2.6) |

- **Steps:** 7.8 to 8.3 a ray against 8.9 to 9.9, every run; sky rays 1.3 to
  2.4 steps against 2.3 to 4.0. Sky tiles do pay in game at 64 steps, on
  the tiles high enough to reach the tree's edge in time (a run with 1,024
  steps brought sky rays to 0.4 to 0.5 steps, but its beam pass sampled at
  5 to 6 ms mean).
- **Trace:** sampled minimum 4.2 to 5.2 ms against 5.0 to 6.5, mean 6.6 to
  9.5 against 8.8 to 11.7.
- **Beam pass:** minimum 2.2 to 2.4 ms against 1.3 to 1.9, now carrying the
  head rows.
- **Sum of minimums** (beam plus trace, per frame line): 6.4 to 7.4 ms
  against 6.6 to 8.3; of means 9.8 to 14.0 against 11.6 to 14.7. The
  sampled timer overstates short passes on this GPU (decision 0015) and
  these runs were CPU-starved, so the bench's 0.4 to 0.5 ms is the better
  figure; the walk agrees in direction and in steps.

Unsampled (sampling off, `frame times, steady` over frames 101 to 3000; mean
/ p95 ms), in pairs, the last two pairs in reverse order:

| Pair | base | new | Loads (base; new) |
|---|---|---|---|
| 1 | 23.2 / 37.2 | 22.3 / 34.8 | 122-128; 124-112 |
| 2 | 22.8 / 36.4 | 22.0 / 35.2 | 112-113; 113-121 |
| 3 | 21.5 / 34.1 | 19.1 / 29.1 | 121-92; 92-40 |
| 4 (new first) | 21.1 / 35.7 | 19.7 / 31.5 | 81-34; 31-81 |
| 5 (new first) | 22.3 / 35.3 | 21.9 / 37.9 | 53-77; 34-53 |

The new build's mean was lower in all five pairs, by 0.4 to 2.4 ms, but
the frames were CPU-bound at a third of the CPU's speed and the spread
between runs of one build (19.1 to 22.3 ms new, 21.1 to 23.2 base) is as
large as the difference; p95 went either way. The walk does not show a
frame-time gain that noise cannot explain, and the plan's bar (no frame
over 16 ms) was not reached by either build under these conditions.

## Alternatives considered

- **Folding the beam into the trace's workgroups** (each 8x8 workgroup is
  one tile: one lane marches, the others wait at a barrier, then all
  trace). It removes the separate pass, but a march is up to 64 steps,
  each a chain of dependent node loads, much longer than one workgroup's
  trace, and every workgroup would hold its slot through it: at 720p's
  14,400 workgroups that runs in many rounds the marches the separate pass
  runs all at once. Not built.
- **Overlapping the prepass with other passes.** Nothing in the frame is
  independent of it: the brick cache's apply and eviction passes write the
  nodes it reads, and the shade and composite read the trace's output.
  Bevy's opaque pass, recorded between them, draws only Bevy's own meshes. Only
  the trace's own rays are independent of the beams of other tiles, which
  is what the head band uses.
- **4x4 sub-beams after the 8x8 march** (each tile's thread continues four
  narrower beams from where its own stopped, written at 4-pixel
  granularity): decision 0023 measured 4x4 tiles from the camera as a loss
  (the prepass costs more than the trace saves). The look into occupied
  cells takes the trace to 4.9 ms on the same scene, below the 5.2 ms
  decision 0023 measured with 4x4 tiles, without more threads or a larger
  buffer, so the sub-beams were not built.
- **More beam steps for sky:** 256 or 1,024 steps prove most sky tiles in
  game (sky rays at 0.4 to 0.5 steps each instead of 2.4 to 4), but the
  prepass's longest tiles cost more than the trace saves (bench above, and
  sampled beam times of 4 to 6 ms in game).
- **Head band at the top of the image:** sky rays traced from the camera
  without a beam take 11 to 16 steps each; the bottom rows' rays end near
  the camera and take few.

## Consequences

- The prepass and trace take about 0.4 to 0.5 ms less GPU time on the
  bench; on the floor walk steps per ray fall from 8.9-9.9 to 7.8-8.3 and the
  trace's sampled minimum from 5.0-6.5 to 4.2-5.2 ms, while frame times did
  not change beyond noise on a CPU-starved machine. The plan's
  bar (no frame over 16 ms on the floor machine) is still not met; the
  trace with its prepass remains the largest pass.
- The prepass dispatch is one-dimensional and also traces; `record_beam`
  takes the head rows, and the trace in camera mode now requires the
  prepass to have run (it always did for its start distances). The sampled
  timer's `beam` pass includes the head rows' trace, and its `trace` pass
  no longer does, so neither is comparable with decision 0023's figures on
  its own; their sum is.
- `TraceParams` gains `head`; `brick_cache.ron` gains
  `lod.beam_head_share` (0.125; 0 gives decision 0023's split).
- Exactness also rests on the two arguments above, guarded by `gpu_beam`.
  Any change to the trace's stop conditions must still be mirrored in the
  beam's occupied cases, now including what the look into a child assumes:
  that a clear bit of any non-uniform node is an empty cell with no side
  effects, and that an internal node at level 2 or more is descended into
  unless the footprint test fires.
- Next ideas: a cheaper sky proof (a per-frame summary of the highest
  occupied cell per coarse column would let a cube that is above it and
  rising be called sky without marching to the tree's edge); a second look
  (two levels) where the first look is clear but its box is small; reusing
  the beam for shadow rays (one bundle per tile towards the sun); and the
  shade pass (3 ms sampled), now the next largest after the trace.
