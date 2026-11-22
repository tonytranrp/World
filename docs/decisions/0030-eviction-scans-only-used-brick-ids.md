# 0030: The eviction pass scans only brick ids ever handed out

Status: accepted, 2026-11-21

## Context

The eviction pass (decision 0007) is one 256-thread workgroup that scans
every entry of the brick table twice, once for an age histogram and once to
collect victims. The table has an entry per brick id the pool could hold:
786,432 on the floor profile, about two million on the PC profile. The
render baseline on the Windows laptop
(`docs/research/perf-baseline-windows-render.md`) found 30,000 to 90,000
ids in use, and the pass at 4.2 and 7.4 ms of GPU time each time it ran,
a dropped frame each.

## Decision

`IdSpace` (`brick_cache/pool/alloc.rs`) tracks the high water, one more
than the largest id it ever handed out, and the pass's `id_count` is that
instead of the table's size (`Residency::brick_id_high_water`). Ids at or
above it were never allocated: their table entries are zero, which the pass
already skips as free, so it chooses exactly the same victims. Freed ids go
back on top of the free stack and are handed out first, so the high water
stays at the most bricks ever resident at once.

## Measured

The four alternating floor walks of decision 0028 (`--mac-floor
--exit-after-frames 3000 --walk 8 --backend vulkan`, RX 6800M, timestamps),
every frame the pass ran:

| Build | Frames | Eviction pass, ms |
|---|---|---|
| base | 1 | 2.84 |
| base | 2 | 2.85 to 5.28 (mean 4.06) |
| new | 4 | 0.19 to 0.36 (mean 0.24) |
| new | 2 | 0.16 to 0.34 (mean 0.25) |

About 15 times cheaper. The default-profile walks did not evict.

## Alternatives considered

- **Several workgroups** (a histogram pass, then a compaction pass whose
  workgroups each pick the threshold): needs a second dispatch and a global
  histogram. Worth it if the pool ever holds hundreds of thousands of
  bricks at once; with the high water the single workgroup scans tens of
  thousands of ids.
- **Lowest free id first** (a sorted free list or bitmap): would keep the
  high water at the current count rather than the peak, at a cost on every
  allocation. The peak is close enough on the walks.

## Consequences

- A walk that once filled the pool keeps scanning up to that peak until
  restart.
- The CPU model `choose_victims` already takes `id_count`; tests are
  unchanged.
