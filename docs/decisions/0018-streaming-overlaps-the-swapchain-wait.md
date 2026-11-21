# 0018: Where the CPU frame goes, and streaming overlaps the swapchain wait

Status: accepted, 2026-11-21

## Context

After decision 0015's whole-node skips the trace on the floor machine (2019
MacBook Pro, i7-9750H, Radeon Pro 5300M, Metal, `--mac-floor`) takes about
9 ms at its minimum, yet release walks still averaged 20.5 to 23 ms a frame
with the GPU busy only 70 to 75% of the time. So part of the frame was not
GPU work. The question was where the main thread and the render thread
spend each frame, and what to cut.

Bevy renders pipelined: the main thread runs the main schedules, then
extracts into the render world, which first waits until the render thread
has finished the previous frame; the render thread runs `Render`. The
frame's period is therefore the slower of the two threads plus extraction.
On the render thread, `prepare_windows` (`RenderSystems::PrepareViews`)
calls `get_current_texture`, which blocks until the swapchain has a free
drawable: with Bevy's defaults (`PresentMode::AutoVsync`, which is FIFO on
Metal, and a maximum frame latency of 2, so three drawables) that wait is
for the GPU to finish and present earlier frames.

## Where the frame went

Timing was taken with temporary `Instant` probes (not committed): a probe
schedule between each of the main schedules, probe systems at the
boundaries of the `Render` sets and the `RenderGraph` sets, wrappers around
both extract functions, and timers around the brick cache's work, averaged
over frames 101 to 2100 of a 3,000-frame 8 m/s walk (release build,
`cargo build --release -p world`, sampling off). The probed binary's frames
matched the unprobed one's (21.5 against 21.4 ms mean, run back to back).
Load average 8, CPU speed limit 100%:

| Main thread | ms a frame |
|---|---|
| First, PreUpdate, StateTransition | 0.9 |
| RunFixedMainLoop (physics) | 1.4 |
| Update (residency `update_world` 0.6, `feed_renderer` under 0.01) | 0.9 |
| PostUpdate, Last, SpawnScene | 1.2 |
| Waiting for the render thread, then extracting (extract 0.23) | 16.9 |

| Render thread | ms a frame |
|---|---|
| Waiting for the main thread | 0.4 |
| Extract commands, assets, views, queue, sort | 0.3 |
| `PrepareViews`: `prepare_windows` waiting for a drawable | 6.6 |
| `Prepare` | 1.3 |
| `RenderGraph` / `Begin`: `run_brick_cache` | 10.5 |
| of which `run_frame` (stage 1's `select_split` 2.6, applying changes 3.3, expanding 2.2, loading 0.6, collapsing 0.5) | 9.6 |
| of which collecting read-backs | 0.4 |
| of which serializing, staging and recording the passes | 0.4 |
| Cameras (recording trace, shade, composite) | 0.4 |
| Submit | 1.2 |
| Present and cleanup | 0.6 |
| Total (the frame) | 21.4 |

The main thread works about 4.4 ms and waits the rest: **the render thread
sets the frame**, and on it the brick cache's CPU streaming is the largest
item, 10.5 ms, with the drawable wait next at 6.6 ms. They ran one after
the other: the render thread waited for a drawable, so the GPU was busy or
idle at a present boundary, and then spent 10 ms on the CPU before
recording anything for the GPU. Each frame's GPU work could only start once
the CPU work of that same frame was done, and the CPU work of the next
frame only after the drawable wait. Nothing else on either thread was over
1.5 ms. Per-frame allocation and the residency systems on the main thread
are not where the time goes.

## Decision

Split the brick cache's frame in two (`brick_cache/render.rs`):

- **`stream_brick_cache`** does the CPU part: collect finished read-backs,
  take the feed, check the apply and evict pipelines are compiled, and run
  `run_frame` into the upload batch. It runs in the `Render` schedule in no
  set, chained after `init_brick_cache`, `advance_frame` and
  `update_view_point`, ordered after `RenderSystems::ExtractCommands` and
  before `RenderSystems::Render`. Bevy 0.20 chains `Render`'s sets only
  weakly (an edge holds only between systems whose accesses conflict), and
  the cache's systems conflict with nothing `prepare_windows` touches, so
  streaming runs on a task pool thread while `prepare_windows` waits for
  the drawable. In `RenderSystems::Prepare` it could still have been
  ordered behind `prepare_windows` through a conflicting or deferred system
  in a set in between; in no set it has only the edges written here.
- **`run_brick_cache`** (`RenderGraph` / `Begin`, unchanged place) records
  what the stream produced: serialize the batch, stage the upload, record
  the apply pass and, when asked, the eviction pass. A frame whose stream
  did not run (shaders compiling, no world) records nothing, as before.
- `prepare_view_targets` is now ordered after `advance_frame`. It writes
  the frame number into the trace parameters; before, the two were
  unordered (both in `Prepare`), so the trace could stamp bricks with the
  previous frame's number.

`run_frame` gets exactly the inputs it got before (the same feed, view
point, frame number, read-backs and config), so what it does is unchanged
and stays deterministic, and every count budget of decision 0012 holds.
Only its place in the render thread's frame moved.

## Measured

Steady frames (101 to 3000) of 3,000-frame walks at 8 m/s, `--mac-floor`,
release, sampling off, alternating builds. `pmset -g therm` showed
`CPU_Speed_Limit` 100 before and after every run; load averages are the
1-minute figures before and after each run. GPU busy is the AMD GPU's
`GPU Activity(%)` from the IORegistry once a second, after the first 10 s.

| Run | Load | Mean / p50 / p95 / p99 / max | Frames over 17.5 / 25 / 34 ms | GPU busy |
|---|---|---|---|---|
| before | 6.0-7.3 | 21.4 / 19.2 / 39.0 / 46.1 / 55.2 ms | 58 / 29 / 11% | 72% |
| after | 6.6-7.4 | 17.2 / 16.7 / 22.4 / 32.4 / 83.6 ms | 21 / 3.0 / 0.4% | 79% |
| before | 7.1-7.0 | 19.2 / 17.6 / 34.2 / 41.9 / 56.9 ms | 51 / 18 / 5.2% | 79% |
| after | 6.9-9.0 | 17.5 / 16.7 / 24.0 / 33.7 / 56.7 ms | 23 / 4.4 / 1.0% | 80% |
| before | 7.9-7.4 | 20.4 / 18.0 / 36.9 / 44.9 / 172.0 ms | 53 / 24 / 8.4% | 73% |
| after (an hour later) | 5.5-7.1 | 17.4 / 16.7 / 24.0 / 31.6 / 96.3 ms | 29 / 4.1 / 0.5% | 80% |
| before | 7.1-7.5 | 21.0 / 18.6 / 38.2 / 44.9 / 116.2 ms | 56 / 27 / 9.7% | 72% |

The mean falls by 1.7 to 4.2 ms (19.2 to 21.4 ms before, 17.2 to 17.5
after), the median sits on the display's 16.7 ms, and p95 roughly halves.
The worst single frame is not better (56.7 to 96.3 ms after, 55.2 to 172
ms before): a long streaming frame still costs its whole length.
The machine was quiet for these runs (load 5 to 9, against 15 to 380 in
decisions 0015 and 0017), and the before runs differ among themselves by
2.2 ms of mean, so the size of the gain is known to about that; its sign
and the change of shape (median on vsync, 21-29% of frames over 17.5 ms
against 51-58%) are well outside the noise. Over the same 3,000 frames the
faster build walks less far (416-422 m against 467-523 m), so the cache at
frame 2,000 is comparable (1.41 million records and 42,000 bricks after,
1.27 million and 34,000 before).

With the probes put back on the new build (load 4.6 to 7.2), the render
thread's frame was 17.0 ms: `PrepareViews` 12.0 ms (the drawable wait and,
in parallel, streaming at 8.1 ms), `Prepare` 1.2, `Begin` 0.4 (recording
only), cameras 0.35, submit 1.7, present and cleanup 0.3; the main thread
still worked about 4.4 ms and waited 12.3.

Where the remaining slow frames come from, per render frame of a second
probed walk (2,897 frames, mean 17.7 ms):

| Render frames | Share | `run_frame` | `select_split` | applying changes | expanding | loading |
|---|---|---|---|---|---|---|
| up to 17.5 ms | 73% | 6.5 ms | 1.8 | 1.9 | 1.8 | 0.2 |
| 17.5 to 25 ms | 21% | 11.9 ms | 3.7 | 3.6 | 2.5 | 1.1 |
| over 25 ms | 6% | 16.0 ms | 4.2 | 5.3 | 3.1 | 1.8 |

562 of the 787 frames over 17.5 ms had `run_frame` over 10 ms. The frame
now misses vsync mostly when streaming takes longer than the drawable wait
plus what follows it allows, about 11 ms.

## Alternatives considered

- **Streaming in `RenderSystems::Prepare`.** The smallest change, but the
  weak chain still orders the cache's systems behind any conflicting or
  deferred system of the sets before it, and whether one of those comes
  after `prepare_windows` depends on Bevy's own systems. Putting them in no
  set states the only orderings they need.
- **Streaming on a worker thread across frames** (start frame N+1's
  `run_frame` as soon as frame N is recorded, with N's feed). It would also
  hide streaming behind the main thread's update, but the feed for N+1
  only exists after N+1's extraction, so it adds a frame of latency to
  every edit and swapped chunk, and the residency would be shared between
  a background task and the render world. Worth it only if streaming stays
  longer than the drawable wait.
- **`PresentMode::AutoNoVsync` or a larger frame latency.** Changing the
  present mode trades tearing or latency for throughput and leaves the
  CPU and GPU still serialized inside the frame; the overlap fixes the
  cause instead. Left at Bevy's defaults.
- **Lowering stage 1's per-frame budgets** (`max_visits_per_frame`,
  `max_summaries_per_frame`, `max_expansions_per_frame`). It would trim the
  slow frames above directly, at the cost of the cache refining more
  slowly on a walk; it is a tuning question for `brick_cache.ron` with its
  own quality check, not part of this change.

## Consequences

- On the floor machine the walk now runs at the display's rate for most
  frames; the remaining cost above 16.7 ms is the brick cache's CPU work
  on frames where it runs long, and the GPU, now 79-80% busy.
- `stream_brick_cache` runs while other render systems do, on Bevy's task
  pool; `run_frame`'s own parallel parts still use the brick cache's pool.
  Systems that read `BrickCacheState` (`prepare_view_targets`) are ordered
  against it only by their conflicting access, which Bevy serializes in
  either order; nothing they read changes in `run_frame`.
- Measured with the default `cargo build --release -p world`, which keeps
  the `dev` feature (dynamic linking, the asset watcher and the egui
  panels). A `--no-default-features` build was not measured.
- Next: shorten the long streaming frames (applying changes and stage 1's
  growth pass together are 7 to 9 ms of them), drop the request block's
  copy out of the read-back while stage 1 ignores it, and look at the
  1.7 ms submit.
