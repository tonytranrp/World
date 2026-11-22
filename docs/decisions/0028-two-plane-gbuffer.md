# 0028: Camera traces write a 16-byte G-buffer in two planes

Status: accepted, 2026-11-21

## Context

The render baseline on the Windows laptop
(`docs/research/perf-baseline-windows-render.md`, RX 6800M, Vulkan) has the
trace writing a 32-byte hit record per pixel: hit voxel, packed word, `t`,
depth, steps and a zero word. The shade pass, its only reader in the game,
needs the packed word and the depth, 8 bytes, but read whole 32-byte
records through the cache. The hit voxel (words 0 to 2) is only compared by
tests. Decision 0017 measured the shade pass as bound by this traffic.

## Decision

A camera trace (`MODE_CAMERA`) of `n` pixels writes two planes of 2 words
per pixel (`gbuffer`, `passes/primary/trace.wesl`):

| Plane | Words | Content |
|---|---|---|
| shading | `0 .. 2n` | packed word (material, level, face, flags), depth |
| ray | `2n .. 4n` | `t`, steps |

The shade pass reads only the shading plane (8 bytes a pixel, neighbouring
pixels in neighbouring words); the sampled ray statistics read both
(`gbuffer::camera_records`). The ray plane keeps `t` for later lighting.

Rays read from the buffer (`MODE_RAYS`, the CPU comparisons) keep the full
32-byte record, which also carries their input ray. A new
`MODE_CAMERA_FULL` traces camera rays into full records, so `gpu_beam`
still compares every hit voxel bit for bit with and without the beam. The
new test `gpu_camera_planes_hold_the_full_records_fields` checks that the
two planes hold exactly the packed word, depth, `t` and steps of the full
records, with the beam and its head rows on as in the game.
`HIT_WORDS` (4, both planes) sizes the game's buffer, `HIT_RECORD_WORDS` (8)
is the full record, and `HIT_PLANE_WORDS` (2) is generated for the shaders
(decision 0013).

## Measured

Alternating walks (`--exit-after-frames 3000 --walk 8 --backend vulkan`,
release), the base commit `24f61e9` (base) and this change (new), two pairs
per profile; `gpu passes` lines at frames 1000 and 2000 (timestamps, ms
mean / min for shade, mean for the trace).

| Profile, pair | Shade base | Shade new | Trace base | Trace new |
|---|---|---|---|---|
| floor 1 | 0.09 / 0.08, 0.09 / 0.08 | 0.06 / 0.05, 0.06 / 0.05 | 1.66, 1.74 | 1.67, 1.74 |
| floor 2 | 0.10 / 0.08, 0.09 / 0.08 | 0.06 / 0.05, 0.06 / 0.05 | 1.96, 2.01 | 1.76, 1.87 |
| default 1 | 0.25 / 0.24, 0.25 / 0.24 | 0.14 / 0.12, 0.13 / 0.13 | 3.50, 4.16 | 3.99, 4.10 |
| default 2 | 0.23 / 0.20, 0.23 / 0.22 | 0.13 / 0.12, 0.13 / 0.13 | 3.65, 4.10 | 3.45, 4.03 |

- **Shade: -35% on the floor profile, -45% on the default one** (0.09-0.10
  to 0.06 ms, 0.23-0.25 to 0.13-0.14 ms).
- **The trace** writes half the bytes; its time is unchanged within the
  noise (it is bound by dependent node loads).
- **GPU memory:** the G-buffer halves, 14.7 to 7.4 MB at 1280x720, 59 to
  29.5 MB at 2560x1440.
- Frame times (steady mean / p95, ms) did not move beyond noise: floor 6.4 /
  8.9 and 7.5 / 12.3 base, 6.6 / 10.1 and 7.0 / 11.2 new; default 9.6 /
  13.5 and 9.0 / 12.4 base, 9.9 / 15.6 and 8.9 / 12.0 new. The saving is
  0.03 to 0.1 ms of a 6 to 10 ms frame; it is larger on the floor Mac,
  whose shade pass decision 0017 measured at 0.35 ms back to back.

## Alternatives considered

- **One interleaved 16-byte record** (packed word, depth, `t`, steps): the
  same writes, but the shade pass would still read 16 bytes a pixel.
- **An 8-byte G-buffer without the ray plane:** the shade pass's traffic is
  the same; the ray plane costs 8 bytes a pixel of writes (about 0.03 ms
  here) and keeps `t` and the step statistics.
- **A smaller lit target** (`Rgba32Float`, written by shade and read by the
  composite): not changed. Decision 0017 keeps full `f32` so the composite
  writes bit-identical colours, and no golden image draws the composite, so
  nothing would catch a rounding change; the composite costs 0.06 to 0.10
  ms here.

## Consequences

- Tests that need hit voxels trace in `MODE_RAYS` or `MODE_CAMERA_FULL`;
  the camera mode's mode check in `view.wesl` is now "not `MODE_RAYS`".
- Later lighting reads the shading plane and, for positions, `t` from the
  ray plane or the depth.
