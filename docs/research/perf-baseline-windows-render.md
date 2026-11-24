# Render performance baseline on the Windows laptop

Measured 2026-11-23 on `perf/integration` at f65d385 (decisions up to 0026),
before the `perf/render` changes, as the reference the render optimizations
compare with.

Machine: Ryzen 9 5980HX (8 cores), 15 GB RAM, AMD Radeon RX 6800M (driver
24.10.1, Vulkan tier A, DX12 tier B) beside an integrated Radeon. Two other
agents' builds ran throughout, so CPU-side figures (frame times) are noisy;
GPU pass times come from timestamp queries and are the steadier figure.

## Bench (`WORLD_TEST_ADAPTER=gpu cargo bench -p world_render --bench gpu_timing`)

RX 6800M, Vulkan, timestamps for single passes, wall time for the beam and the
trace in one submission; medians of 20 runs. "64/11" is the game's default
(64 beam steps, 11 head rows).

| Scene | Config | Beam | Trace | Both, one submission (wall) |
|---|---|---|---|---|
| hills | no beam | - | 2.55 | 2.69 |
| hills | 16/0 | 0.45 | 1.68 | 2.27 |
| hills | 64/0 | 0.82 | 1.50 | 2.46 |
| hills | 64/11 | 0.84 | 1.32 | 2.31-2.35 |
| hills | 256/11 | 0.85 | 1.32 | 2.32 |
| horizon | no beam | - | 2.41 | 2.56 |
| horizon | 16/0 | 0.44 | 1.65 | 2.24 |
| horizon | 64/0 | 0.85 | 1.44 | 2.43 |
| horizon | 64/11 | 0.89 | 1.08 | 2.12 |
| horizon | 256/11 | 1.89 | 1.03 | 3.06-3.09 |

(Full resolution and footprint bias 1 give the same figures within 0.04 ms.)
Apply: 256 bricks 0.20 ms, 2,048 bricks 0.35 ms, 2,560 bricks 0.45 ms (wall).

On this GPU the beam saves less than on the floor Mac: the prepass costs a
third of the trace it shortens, and the two passes together are only 0.3 to
0.4 ms below the trace without a beam.

## In-game walk

`world --exit-after-frames 3000 --walk 8 --frame-times <file> --backend
vulkan [--mac-floor]`, release build (`--no-default-features`). Frame times
over frames 101-3000; GPU passes from the `gpu passes` lines at frames 1000
and 2000 (timestamps, ms mean / min / max).

### `--mac-floor` (tier B, 1280x720 internal, 64 MiB nodes, 768 MiB bricks)

| Run | Mean | p50 | p95 | p99 | Max | Frames over 16.7 ms |
|---|---|---|---|---|---|---|
| 1 | 9.3 | 8.8 | 14.4 | 16.4 | 46.8 | 24 |
| 2 | 11.7 | 12.0 | 16.3 | 19.0 | 24.0 | 115 |

| Run, frame | Beam | Trace | Shade | Composite | Evict | Sum |
|---|---|---|---|---|---|---|
| 1, 1000 | 2.66 / 1.45 / 5.44 | 2.42 / 1.06 / 5.63 | 0.11 | 0.09 | - | 5.28 |
| 1, 2000 | 2.26 / 1.30 / 5.35 | 2.32 / 1.45 / 5.49 | 0.10 | 0.08 | 4.20 (1 frame) | 4.77 |
| 2, 1000 | 3.33 / 1.47 / 5.47 | 2.96 / 1.09 / 5.23 | 0.12 | 0.11 | 7.36 (2 frames) | 6.55 |
| 2, 2000 | 2.80 / 1.49 / 5.46 | 2.78 / 1.57 / 5.47 | 0.11 | 0.09 | - | 5.79 |

Residency: 1.20 to 1.43 million records, 71,000 to 85,000 expanded, 27,000
to 43,000 bricks (7 to 9 MiB of blocks, 1% of the brick pool), node pool 80 to
96% full (the detail multiplier rose to 1.08 to 1.25 under that pressure).

### Default profile (tier A, viewport-sized trace, 128 MiB nodes, 2 GiB bricks)

| Run | Mean | p50 | p95 | p99 | Max | Frames over 16.7 ms |
|---|---|---|---|---|---|---|
| 2 | 14.6 | 11.7 | 24.4 | 30.3 | 42.5 | 1,108 |

| Frame | Beam | Trace | Shade | Composite | Sum |
|---|---|---|---|---|---|
| 1000 | 2.90 / 1.70 / 6.64 | 4.70 / 2.49 / 11.25 | 0.34 | 0.13 | 8.08 |
| 2000 | 2.92 / 1.67 / 7.03 | 5.13 / 3.26 / 11.44 | 0.36 | 0.14 | 8.55 |

1.27 to 1.53 million records, 84,000 to 87,000 bricks (19 to 20 MiB), node
pool 43 to 52% full.

### Observations

- **The beam and the trace are 90% of the GPU frame** on both profiles.
  In game the beam pass (which also traces the bottom 11 tile rows) costs as
  much as the trace on the floor profile.
- **The eviction pass costs 4 to 7 ms** when it runs: one 256-thread
  workgroup scans all 786,432 brick ids twice although only 30,000 to 45,000
  are in use. It runs a few times per 1,000 frames, each a dropped frame.
- **Shade and composite are small** (0.1 to 0.36 ms): the per-pixel
  bandwidth items matter for the floor Mac more than for this GPU.
- **Residency records** are 1.2 to 1.5 million at 112 bytes each plus about
  18 bytes of table: about 160 to 200 MiB of CPU memory.
- The first default-profile run (not tabled) alternated 250 ms and 14 ms
  frames throughout (mean 131 ms) with GPU passes at their usual times: a
  presentation stall of the unfocused window, not render work. The repeat
  did not show it. Frame-time figures from unattended runs need that check.
- **DX12** (`--backend dx12 --mac-floor`): in a 3,000-frame run the voxel
  passes never ran (0 records, no beam, trace or shade timings, only the
  composite): the trace pipeline was not ready within the run, most likely
  still compiling under FXC/DXC. Not investigated further here.
