# 0017: Voxel lighting at the internal resolution, and no MSAA on the player camera

Status: accepted, 2026-11-20

## Context

Decision 0015 measured the voxel composite at 1.25 to 3 ms on the floor
machine (2019 MacBook Pro, Radeon Pro 5300M, Metal, `--mac-floor`). The trace
runs at the internal resolution (1280x720 in the floor profile), but the
composite ran at the window's full Retina size (2560x1440 for the default
1280x720-point window): per screen pixel it read the hit record under it,
looked up the material colour, multiplied by a light per face and wrote
colour and depth. Every screen pixel over the same traced pixel did that
work again, four times over at 2x.

The player camera also had Bevy's default MSAA (four samples, `Msaa` is a
required component of `Camera`). The voxels arrive as one fullscreen
triangle, so every sample of a pixel gets the same colour and depth; the
four samples only multiplied the composite's colour and depth writes at
Retina size and added a resolve at the end of the pass. The offscreen test
camera already turned MSAA off.

## Decision

### Lighting writes a lit target at the trace's size (`passes::lighting::shade`)

- A compute pass after the trace, one thread per traced pixel
  (`assets/shaders/passes/lighting/shade.wesl`), reads the hit record and
  writes the lit target: `Rgba32Float` at the trace's size, the linear
  colour (material colour times the face light) in rgb and the hit's depth
  (reverse Z, as Bevy's depth texture stores it) in alpha, or alpha -1 for a
  miss.
- The voxel composite (`passes::composite::voxel`) no longer shades. Per
  screen pixel it loads the lit texel under it (the same nearest mapping as
  before, `min(screen * size / viewport, size - 1)`), discards a miss, and
  writes the colour with alpha 0 and the depth. Its per-pixel cost is one
  texture load and the two writes.
- Full `f32` keeps the image bit-identical: the colour and depth the
  composite writes are the same `f32` values it computed before, so every
  target format (the 8-bit sRGB of the tests and the game, or a 16-bit
  float HDR target) receives exactly what it did. It also puts colour and
  depth in one load for the composite.
- The material colour buffer moved with the shading, from the composite to
  `lighting::shade`.
- `TimedPass::Shade` times the pass (`shade` in the `gpu passes` log line,
  between trace and composite).
- This is where lighting v1 and v2 (`shadow`, `ao`, `gi`) will write: their
  cost then scales with the internal resolution, and the composite stays a
  stretch.

### MSAA off on the player camera (`World_Game::player`)

`Msaa::Off` on the camera the player looks through. Nothing rasterized
needs antialiasing yet; when something does (gizmos, debris proxies), it
should get its own answer rather than four samples of the whole voxel image.

## Exactness

The golden files do not change: none of them draws the voxel composite
(`clear_colour.png` and `sky_composite.png` have no voxels, `wall.hits.txt`
is hit records). So two checks were added or run instead:

- `gpu_the_composite_stretches_a_smaller_trace_over_the_viewport`
  (`World_Render/tests/gpu_render.rs`) renders the debug scene at 320x180
  with the trace capped at 213x120 (a ratio that is not whole on either
  axis), without the sky. It checks that every screen pixel over one traced
  pixel has the same colour, that every covered pixel is a material colour
  times a face light (within one 8-bit step of the CPU's sRGB encoding), and
  that misses keep the clear.
- In-game screenshots at 2560x1440 (`--screenshot`, frame 2950 of a
  stationary `--mac-floor` run): the old build (five runs), the new build
  with MSAA on (four runs) and the new build with MSAA off (one run), all at
  load averages 12 to 88, are byte-identical. Under heavier load (210 to
  260) every build, the old one included, differs from those images in
  13572 to 28380 bytes of 11 million, all in rows 630 to 809 (the band
  around the horizon at row 720), whole 2x2 blocks changing material: far
  terrain whose residency had not settled by frame 2950 while worker
  threads were starved. Two old-build runs under that load matched each
  other exactly, and the MSAA-off build's differences under load fall at
  the same bytes (all but 936 and 2556 bytes, those also in rows 720 to
  809). MSAA off is exact by construction as well: every sample of a pixel
  was written with the same value, so the resolve averaged identical
  samples.

## Measured (floor machine, mac-floor, 2560x1440 window, 1280x720 trace)

The machine was shared with other agents' builds and tests throughout. Load
averages ran from 15 to 380 and `pmset -g therm` showed `CPU_Speed_Limit`
between 33 and 100%. Each table states its conditions.

### Back-to-back GPU time (the clearest numbers)

A temporary bench on bare wgpu (not committed) ran the old and the new
composite shader with the same pipeline state as the game (fullscreen
triangle, `GreaterEqual` depth test writing `frag_depth`, 8-bit sRGB colour,
`Depth32Float`, a resolve target for MSAA), 1280x720 synthetic hit records
(55% hits, four materials, all faces) stretched to 2560x1440, and the real
`shade.wesl`. Each sample records 20 passes in one command buffer and
divides the wall time, so submission latency and the GPU's clock ramp after
an idle wait drop out; median of 15 samples. Runs 1 to 3 at load 58 to 61,
run 4 half an hour later at load about 50, CPU speed limit 100% for all:

| Pass | Run 1 | Run 2 | Run 3 | Run 4 |
|---|---|---|---|---|
| old composite, MSAA 4 | 0.802 | 0.815 | 0.805 | 0.755 ms |
| old composite, MSAA off | 0.383 | 0.382 | 0.379 | 0.342 ms |
| new composite, MSAA 4 | 0.734 | 0.740 | 0.740 | 0.688 ms |
| new composite, MSAA off | 0.323 | 0.330 | 0.326 | 0.285 ms |
| shade, 1280x720 | 0.349 | 0.355 | 0.345 | 0.318 ms |

Per frame, before (old composite, MSAA 4) against after (shade plus new
composite, MSAA off): 0.80 against 0.68 ms. The two changes pull in
opposite directions:

- **MSAA off saves about 0.42 ms** of composite alone (0.80 to 0.38 with the
  old shader, 0.74 to 0.33 with the new). Bevy's opaque pass also stops
  clearing and resolving a four-sample colour and depth target at Retina
  size, which this bench does not count.
- **Shading at the internal resolution costs about 0.3 ms today.** The new
  composite is only 0.05 ms faster than the old one at the same sample
  count, and the shade pass adds 0.35. With shading this cheap (a colour
  lookup and a multiply), both passes are bound by memory traffic, not
  arithmetic: the old composite reads the hit records (32-byte records, so
  about 29.5 MB of them for 7.4 MB of used words) and writes 29.5 MB of
  colour and depth; the new path adds the lit target's 14.7 MB written and
  read again. The shade pass runs near 126 GB/s, about what the 5300M
  sustains. The change pays once lighting has real per-pixel work (shadow
  and AO rays, GI lookups), which then runs once per traced pixel instead
  of four times.

### Sampled in-game timing

`sample_every_frames: 60`, stationary scene, the `gpu passes` lines logged
at frames 1000 and 2000 of 3000 (15 and 17 sampled frames). Alternating
runs; conditions are the load average and CPU speed limit before and after
the run; ms mean / min:

| Run | Conditions (load, CPU limit) | Trace | Shade | Composite |
|---|---|---|---|---|
| old, MSAA 4 | 16-12, 100% | 12.3 / 10.1, 12.4 / 12.1 | - | 3.87 / 3.43, 4.02 / 3.54 |
| old, MSAA 4 | 15-38, 100-68% | 12.9 / 9.7, 15.9 / 11.8 | - | 3.76 / 3.33, 3.34 / 1.04 |
| new, MSAA 4 | 36-16, 100-35% | 21.7 / 10.3, 18.2 / 12.0 | 1.47 / 0.43, 2.12 / 0.54 | 1.26 / 0.87, 1.31 / 0.22 |
| old, MSAA 4 | 41-21, 37-35% | 14.9 / 10.5, 15.9 / 12.5 | - | 3.65 / 2.16, 3.92 / 3.44 |
| new, MSAA 4 | 21-88, 35-33% | 12.4 / 10.3, 16.1 / 12.6 | 3.28 / 2.82, 4.21 / 0.00 | 1.57 / 1.44, 2.34 / 1.53 |
| new, MSAA off | 88-216, 33-37% | 14.2 / 9.5, 13.3 / 11.3 | 2.67 / 0.00, 3.11 / 0.00 | 0.91 / 0.79, 1.00 / 0.47 |
| old, MSAA 4 | 216-258, 37-35% | 13.3 / 9.5, 14.9 / 11.0 | - | 6.63 / 3.72, 3.81 / 0.00 |
| new, MSAA 4 | 258-247, 35% | 15.2 / 9.8, 14.3 / 11.5 | 3.45 / 0.00, 3.49 / 2.69 | 2.45 / 0.08, 1.69 / 1.56 |
| new, MSAA off | 247-233, 35% | 13.9 / 9.2, 13.6 / 11.1 | 4.08 / 0.00, 2.52 / 0.00 | 1.13 / 0.85, 0.99 / 0.81 |
| old, MSAA 4 | 236-242, 35-33% | 13.9 / 9.0, 14.4 / 11.1 | - | 4.25 / 0.00, 3.93 / 0.00 |
| new, MSAA 4 | 242-218, 33% | 13.7 / 9.3, 14.6 / 11.1 | 2.79 / 0.00, 3.28 / 0.00 | 1.56 / 1.40, 1.62 / 1.41 |
| new, MSAA off | 218-211, 33-35% | 13.3 / 9.6, 13.4 / 11.6 | 3.56 / 0.00, 3.14 / 0.00 | 1.10 / 0.82, 0.94 / 0.83 |
| old, MSAA 4 | 63-46, 100-93% | 12.9 / 9.2, 17.8 / 11.9 | - | 3.69 / 1.87, 3.90 / 3.40 |
| new, MSAA 4 | 46-50, 93-37% | 11.9 / 9.9, 12.9 / 12.1 | 3.13 / 2.93, 3.02 / 2.66 | 1.45 / 1.39, 1.45 / 1.38 |
| new, MSAA off | 50-37, 37-40% | 12.4 / 9.4, 17.5 / 11.2 | 3.19 / 3.03, 3.03 / 0.00 | 0.89 / 0.70, 0.88 / 0.82 |
| old, MSAA 4 | 52-54, 100% | 13.0 / 9.6, 13.4 / 11.9 | - | 3.73 / 3.28, 3.95 / 3.51 |
| new, MSAA 4 | 54-59, 100% | 16.5 / 10.7, 15.4 / 12.3 | 2.97 / 2.58, 3.06 / 2.75 | 1.46 / 1.36, 1.44 / 1.39 |
| new, MSAA off | 59-24, 100% | 12.3 / 9.7, 12.4 / 12.1 | 3.02 / 2.79, 3.08 / 2.89 | 0.79 / 0.69, 0.84 / 0.70 |

The composite falls from 3.3-4.0 ms (mean, MSAA 4) to 1.3-1.7 with the
stretch-only composite and to 0.8-1.1 with MSAA off as well. The sampled
figures are larger than the bench's because a sampled pass runs alone after
the GPU has idled, and the 5300M drops its clock when idle. The bench shows
it directly: the shade pass submitted alone, minus an empty submission,
takes 0.40 ms right after other work, 0.52 ms after 5 ms idle, and 4.7 and
4.5 ms after 20 and 50 ms idle (run 4). In a sampled frame
the shade pass follows the trace's ray statistics, which read the whole hit
buffer back and walk it on the CPU, the longest idle gap in the frame, so
its sampled time sits at 2.5 to 4.2 ms mean (2.6 to 3.0 ms minimum where
not 0) in every run but the first, against 0.32 to 0.36 ms back to back (0.43 to 0.54 ms minimum in the first, quiet
run, and 0 where the empty-submission subtraction overshot). The
composite, sampled straight after the shade pass, finds the GPU warm. The
sampled shade time and the `sum` column therefore cannot settle the
comparison; the bench does. For the same reason the old composite's
sampled 3.3 to 4 ms, sampled right after the trace's read-back, overstates
its 0.8 ms of work, and decision 0015's 1.25 to 3 ms composite reading had
the same bias. The trace is unchanged within the noise, as it should be.

### Frame times

Sampling off, stationary, `--frame-times`, frames 1001-3000, GPU busy from
the IORegistry's `GPU Activity(%)` once a second. Alternating; load 192 to
379 and CPU speed limit 33 to 44% throughout:

| Run | Mean / p50 / p95 / p99 / max | GPU busy |
|---|---|---|
| old, MSAA 4 | 24.2 / 20.2 / 51.9 / 72.7 / 115.4 ms | 82% |
| new, MSAA 4 | 25.0 / 21.5 / 50.8 / 69.8 / 92.3 ms | 82% |
| new, MSAA off | 24.2 / 20.4 / 50.5 / 78.0 / 137.3 ms | 79% |
| old, MSAA 4 | 23.3 / 21.3 / 44.1 / 53.9 / 77.6 ms | 86% |
| new, MSAA 4 | 24.2 / 20.7 / 49.5 / 67.5 / 138.2 ms | 84% |
| new, MSAA off | 22.8 / 19.4 / 47.0 / 57.8 / 87.6 ms | 80% |

At this load the CPU side of the frame varies by more than the whole
composite costs, so **the frame times show no difference either way**; the
noise hides a 0.1 to 0.5 ms change. They are recorded so a quieter rerun
has something to compare with.

## Alternatives considered

- **Keep shading in the composite and only turn MSAA off.** Cheapest today
  (0.38 ms against 0.68 in the bench). Rejected because every lighting
  stage to come would then run per screen pixel, or the composite would
  have to read and combine their results per screen pixel; the plan has
  lighting produce one lit target at the internal resolution
  (`passes/lighting`, `passes/upscale`). If lighting stays this simple for
  long, this is the variant to return to.
- **A 16-bit float lit target** (half the lit target's traffic, about
  0.1 ms less). Rounding the colour to `f16` and the depth to a separate
  `R32Float` would no longer give the same image bit for bit on an 8-bit
  target, and depth needs 32 bits anyway.
- **Store the shading inputs (material, face, depth: 8 bytes) instead of a
  colour.** Halves the lit target, but the composite would shade again, and
  it stops being the place later lighting writes to.
- **Shade inside the trace pass.** Saves the shade pass's read of the hit
  records (most of its 0.35 ms) at the cost of binding colours and the lit
  target into the trace, which the CPU comparison tests and benches also
  build, and of mixing lighting into primary visibility. Worth revisiting
  if a separate pass stays this expensive relative to its work.
- **Render the whole camera at the internal resolution and upscale to the
  window** (or set the window's scale factor to 1 and let the macOS
  compositor scale the drawable). That would also shrink Bevy's own passes
  (the sky composite, which still runs at Retina size, and the clear), but
  UI would render at 720p, and it changes how the camera, the screenshot
  path and the offscreen tests see their targets. A larger change for the
  upscale stage, not this one.
- **Compute writing straight into the view target.** Depth textures cannot
  be storage textures, and the view target's format is Bevy's choice.

## Consequences

- The voxel image takes one more compute pass and a 14.7 MB lit target per
  view at 1280x720 (`Rgba32Float`). In sustained GPU time the composite
  and shading cost about 0.68 ms a frame on the 5300M at 2x, against 0.80
  before; MSAA off is most of that gain, and shading at the internal
  resolution alone costs about 0.3 ms until lighting does real work there.
- Bevy's opaque pass no longer clears and resolves a four-sample target at
  the window's size (not separately measured).
- The golden images are unchanged. The new GPU test checks the stretch and
  the shading palette, but nothing compares the voxel composite against a
  stored image; an exact image golden of the debug scene would need it to
  stop animating (`debug_scene_edits`) in tests.
- The sky composite (a Bevy fullscreen material) still runs at the window's
  size; it and the voxel composite could become one pass.
