# 0015: GPU time per pass, and rays skip whole empty nodes

Status: accepted, 2026-11-20

## Context

The game logged CPU frame times and brick cache figures but no GPU time, so
nobody could say whether a slow frame on the floor machine (2019 MacBook Pro,
Radeon Pro 5300M, Metal) was the CPU or the GPU. Decision 0009 measured the
trace once, in a test: 7.8 ms at full detail and 1.47 ms with footprint
detail at 1280x720, on the RTX 4070 laptop.

wgpu offers timestamp queries (`TIMESTAMP_QUERY`, written at pass
boundaries). On Metal it reports them only where the GPU can sample
counters at stage boundaries. The 5300M and the Intel UHD 630 next to it
cannot: Metal says they sample at draw, dispatch and blit boundaries only
(`supportsCounterSampling`), and wgpu 30 then reports no timestamp feature
at all. So on the floor machine timestamps cannot time anything. Bevy 0.20's
render diagnostics use the same queries and say outright that they only
record CPU time on Metal.

## Decision

### Timing (`World_Render::gpu_timing`)

- **Timestamps where the device has them, always on.** The trace, the
  voxel composite, the brick cache's apply pass and its eviction pass take
  pass-boundary timestamp writes from a ring of four slots (one query set,
  two queries per pass and slot). After the cameras, the frame's queries are
  resolved and copied into the slot's read-back buffer. The slot is mapped
  after submission and collected two or three frames later without waiting.
  With every slot busy a frame goes untimed and is counted. Each pass is timed
  once per frame (the first 3D view). Cost: eight timestamps, up to four
  resolves and 64 bytes of read-back a frame.
- **Sampling where they are missing, opt-in.** `config/gpu_timing.ron`
  (`sample_every_frames`, 0 by default) makes one frame in that many submit
  each timed pass in its own command buffer and wait for it. The pass's time
  is that wall time minus the wall time of an empty submission. Everything
  recorded before the pass is flushed and waited for first (Bevy's
  `FlushCommands`), so the pass runs alone. A sampled trace also copies the
  hit records back and adds up steps per ray, sky rays, footprint, residency
  and exhausted hits. A sampled frame stalls the render thread, which is why
  sampling is off unless asked for.
- Both report every 1000 frames, next to the `brick cache frame N` line:
  `gpu passes frame N (timestamps|sampled, wall clock alone): ... ms mean /
  min / max (frames)` per pass and for their per-frame sum, and the sampled
  `primary rays frame N` line. Devices without timestamps and with sampling
  off log nothing.

### Traversal: whole-node skips (`traversal/tree.wesl`)

Sampling the trace showed 51 to 54% of primary rays hitting sky, at 46 to 57
steps each, and 54 to 66 steps for the average ray. A 64-tree node is 4
cells a side, so a ray crossing a node it misses can take up to ten steps in
it. Each level of empty space it climbs through costs that much again.

When a ray is in an empty cell (of any node, or a voxel of a leaf), it now
tests the box of cells between that cell and the node's exit against the
node's mask. In the mirrored frame of the traversal the coordinates only
grow, so that box holds every cell the ray could still enter in the node. If
no bit in the box is set, the ray jumps to the node's exit with the same
exact arithmetic it uses to jump an empty cell of the next level up. The
box's far corner is widened by half a voxel plus about eight ulps of the
coordinate, so rounding can only make a skip not happen, never skip a solid
cell. Skipped cells have no side effects: requests and brick stamps come only
from set bits. Hit records are the same apart from the step count, the CPU
comparison tests (`compare_hits`) pass, and the golden images do not change.

## Measured (floor machine, mac-floor, 1280x720 internal)

The machine was thermally limited for most runs (`pmset -g therm`
CPU_Speed_Limit 33 to 100%, mostly near 40%), and another agent's builds kept
the load average between 7 and 85. Each line notes the conditions it ran
under. Sampled times are wall clock minus an empty submission. Their minimum
is the steadier figure, since on a loaded machine the render thread's
wake-up adds to some samples. Composite was never sampled: in these
unattended runs the window had no surface on most frames (Bevy drew no view
target), so the composite pass ran about once in 300 frames. The last run
below had a surface.

GPU pass times before the change, stationary scene after loading,
frames 1001-2000, 20 samples each:

| Build | Trace mean / min / max | Apply mean (frames with one) |
|---|---|---|
| Development | 15.6-22.1 / 12.7-19.5 / 23-31 ms | 0.3-0.5 ms |
| Release | 16.4-17.4 / 13.2-13.3 / 27-28 ms (8 m/s walk) | 0.31-0.35 ms |

Evict ran in no sampled frame (the brick pool stays under its eviction
threshold on these walks).

Release walk (3000 frames at 8 m/s, sampling off): steady frames mean
17.3 ms, p95 31.8, p99 38.7, max 51.5; GPU busy 78% of the time (the AMD
GPU's `GPU Activity(%)` from the IORegistry, once a second). With the trace
alone at 13 to 17 ms of a 17.3 ms frame, **the release frame is GPU-bound**,
and the trace is almost all of the GPU's work. The development build's frames
(28 to 35 ms on the walk) were CPU-bound as well.

Whole-node skips, alternating runs on the stationary scene (development
build):

| Run | Steps a ray (sky rays) | Trace mean / min |
|---|---|---|
| before | 54.2 (49.5) | 16.0 / 14.9 ms |
| after | 20.7 (15.2) | 11.6 / 10.2 ms |
| before | 53.5 | 16.4 / 14.9 ms |
| after | 20.5 | 14.2 / 10.4 ms |
| before | 63.9 (55.8) | 20.6 / 17.9 ms |
| after | 23.8 (17.2) | 18.4 / 12.0 ms |
| before | 65.7 (57.4) | 21.4 / 19.5 ms |
| after | 23.7 (17.0) | 15.5 / 12.3 ms |

Steps fall by 62 to 64% and the trace's minimum by 30 to 37%.

Release walks (3000 frames at 8 m/s, sampling off, alternating, CPU speed
limit 97 to 100%, load 9 to 32):

| Run | Steady mean / p95 / p99 / max | GPU busy |
|---|---|---|
| before | 22.5 / 41.6 / 50.4 / 74.2 ms | 85% |
| after | 20.5 / 37.6 / 45.1 / 53.8 ms | 75% |
| before | 24.4 / 45.0 / 53.9 / 166.4 ms | 77% |
| after | 23.0 / 42.4 / 53.8 / 87.7 ms | 70% |

Sampled release walk after the change: trace 15.1-15.7 / 8.8-9.3 / 22-24 ms
(mean / min / max), against a 13.2-13.3 ms minimum before. In this run the
window had a surface, so the composite was sampled too: 2.6-3.0 / 1.25 /
4.2-4.7 ms. It runs at the window's full Retina size, not the internal
resolution. The frame gains 1.4 to 2 ms rather than the trace's 4: with
less GPU work, more of the frame waits on the CPU (GPU busy fell from 77-85%
to 70-75%).

Tried and dropped, each against the original in alternating runs:

- Caching the brick record, block header and current leaf mask across a
  ray's steps inside a brick: 35% slower (22 against 16 ms), likely from the
  extra live registers lowering occupancy.
- Branches instead of `select` in `step_voxel` (`select` evaluates both
  sides, so the stepping axis's `axis_voxel` loop looked wasted): no change,
  the compiler already dropped it.
- The ancestor stack laid out level-major (`level * 64 + lane`) for
  conflict-free workgroup memory banks: no change.
- Stamping a brick once per ray and reading the leaf count only on a hit: no
  change.
- Skipping up through the ancestors as well (testing each parent's mask
  without the child just left): 23.4 against 23.8 steps a ray, slower in time
  (one more node load per level).

## Alternatives considered

- **Bevy's `RenderDiagnosticsPlugin`.** It needs the same timestamp
  feature, so it times nothing on the floor machine. It also routes results
  through the main world's diagnostics store, where the brick cache's log
  line cannot read them in the render world.
- **Metal command-buffer times (`GPUStartTime`, `GPUEndTime`) through
  wgpu's `as_hal`.** wgpu panics when the raw and wgpu encoding APIs mix on
  one encoder, a finished command buffer has no `as_hal`, and it would need
  unsafe code in a crate that denies it.
- **Instruments' Metal System Trace.** Not available without Xcode, and not
  in the game's log.
- **A beam or coarse depth prepass** to start rays near their hit: a larger
  change with its own exactness questions. It is the next step on the trace.
  Before that, the composite (about 1.3 to 3 ms at Retina size) is a cheaper
  target: it can run at the internal resolution and be upscaled.

## Consequences

- On devices with timestamps (Windows on DX12 or Vulkan, Apple silicon), GPU
  pass times are in every run's log at no measurable cost. On Intel and AMD
  Macs they come only from a sampled measurement run, whose stalls show up
  in the frame times (one frame in `sample_every_frames`), so frame-time
  figures come from runs with sampling off.
- The sampled minimum is a wall-clock figure: it includes a submission's
  latency beyond the empty one and reflects the GPU clock after a short
  idle. Sampled times compare runs on the same machine. They are not
  directly comparable with timestamp figures from another machine.
- `GpuPassTimer`'s resolve and read-back are covered by a GPU test that
  skips itself on devices without timestamps (the floor machine), so here
  only the sampled path was exercised. The timestamp path is unproven until
  that test runs on an adapter with timestamps (CI's Windows adapter, if
  WARP reports them).
- The traversal does more arithmetic per empty cell (three divisions and a
  mask box) to take far fewer steps. Rays that graze solid cells all the way
  gain least.
