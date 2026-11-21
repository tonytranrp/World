# 0016: Column footprints are packed in four bytes and shared, not copied

Status: accepted, 2026-11-20

## Context

Decision 0014's heap audit of the floor walk (2019 MacBook Pro, i7-9750H,
Radeon Pro 5300M, `--mac-floor`, 8 m/s) found World_Gen's column
footprints the largest configured RAM item: the `column_cache_mib` cache
(decision 0011) held 256 MiB, 32 footprints of 8 MiB, and generation
workers held another 24 to 32 MiB of `Vec<Column>` copies, because
`ChunkColumns::for_chunk` cloned a footprint's columns for every chunk
built from the cache.

A `Column` is eight bytes: a 32-bit height, a 16-bit surface material and
two 8-bit depths. What the generator actually puts in them:

| Field | Values |
|---|---|
| Height | the whole shipped terrain spans 22,532 voxels (`height_bounds`: -3,714 to 18,818); 79 surveyed footprints, the 40 steepest of a 48 x 48 coarse survey among them, span at most 2,213 |
| Surface material | one of three: stone (bare rock), dirt (bare soil), grass |
| Cover depth | 0, or `grass_depth_voxels` (2 to 5 by default) |
| Soil depth | 0 to 255 (clamped), 32 to 112 with the default `soil_depth_m` |

## Decision

- **A footprint is stored as one 32-bit word a column**, 4 MiB instead of
  8: from the low bit, the height less a per-footprint base (16 bits), the
  soil depth (8), the surface as an index into bedrock, soil and grass (2)
  and the cover depth (6). The base is the footprint's first column less
  32,768, so a footprint packs when every height is within 32 Ki voxels
  (512 m) of its first, which the whole shipped terrain is.
- **Exactness is checked once per footprint, not per column**: from its
  height range, which the generator already tracks, and from the thickest
  grass the surface rules can give. A footprint that fails (a terrain
  config with more than 512 m of relief inside one 16 m footprint, or grass
  thicker than 63 voxels) is generated again into eight-byte columns. No
  shipped terrain does; a test covers the fallback with 60 to 70 voxel
  grass.
- **The surface rules report the surface as its index** (`Surface::parts`);
  `Surface::column` names the material from it. The words are collected
  straight into the shared allocation (`Arc<[u32]>`), with no intermediate
  vector.
- **Footprints are shared.** `ChunkColumns` holds its columns behind an
  `Arc`, so `for_chunk`, `chunk_source_from` and every chunk source built
  from the cache point at the cache's columns; workers copy nothing.
- **Readers unpack in place.** `ChunkSource::brick` dispatches on the
  storage once per level-1 node; the leaf fill and the min/max pyramid are
  generic over it. A leaf no cave reaches compares its heights from the
  footprint's base, so a packed column costs a mask and two shifts there;
  a leaf caves carve reads whole columns, as before. `ChunkColumns::column`
  returns a `Column` by value; `columns()` (a slice) is replaced by `iter`
  and `to_vec`.
- `bedrock_below` is computed on demand, as before, but over the words: it
  is a 16-bit height less the larger depth, one pass without branches.
- **The default cache halves**, `column_cache_mib` 256 to 128: the same 32
  footprints, the same hit rate. The cache now budgets bytes, counting a
  footprint as what it holds (4 MiB, or 8 if wide) and one still being
  computed as 4.
- Generated voxels do not change: the golden hashes and every terrain test
  pass unchanged, with no new `GeneratorVersion`. Tests changed only where
  they took a `&Column` from a footprint.

## Alternatives considered

- **A narrower height without a base** (16 bits of absolute height). The
  world reaches far beyond 32 Ki voxels; a base per footprint costs four
  bytes per footprint.
- **Three bytes a column** (12-bit height offset, 8-bit soil, 2-bit
  surface, 2-bit cover index). A quarter less again, but unaligned reads
  on the builder's hot path, a height range of only 64 m inside a
  footprint (steep footprints already span 35 m) and a cover index tied to
  four grass depths.
- **Deriving the depths.** The cover depth is a hash of the column and
  the soil depth a smooth noise field; recomputing them per read costs more
  than storing them in 14 bits.
- **Keeping the per-column check.** The first version checked every word
  as it packed it, and computed `bedrock_below` while generating; together
  they made a footprint 16% slower to generate (columns 48.9 against
  56.8 ms in Criterion). Neither is needed per column.
- **Unpacking into a buffer for the readers**, per row of four columns
  or per stack of leaves. Each was slower than reading in place (chunk
  builds from cached columns 5 to 7% slower than before, against 2 to 5%).
- **Only halving `column_cache_mib`** at eight bytes a column: half the
  RAM but half the footprints, so fewer stacked chunks find theirs; the hit
  rate is decision 0011's tuning choice, not overhead.

## Consequences

- Live allocations about 55 s into the floor walk, `MallocStackLogging=lite`
  and `malloc_history -allBySize`, one run each, back to back, release
  build, each binary with its own default (`column_cache_mib` 256 before,
  128 after):

  | | Before | After |
  |---|---|---|
  | Footprints | 256.1 MiB, 32 of 8 MiB | 148.1 MiB, about 37 of 4 MiB |
  | Copies in workers (`for_chunk`) | 24.0 MiB | 0 |
  | Footprint and copy total | 280.1 MiB | 148.8 MiB |
  | Cache hits / computed in the logged interval | 44 / 20 | 60 / 22 |
  | Heap total | 1,445.8 MiB | 1,351.7 MiB |

  The cache itself logged 128 MiB held (32 footprints). The rest, about
  20 MiB, is footprints the cache had already evicted while a worker was
  still building a chunk from them: sharing keeps those alive until the
  chunk is built, where before every such chunk held a private 8 MiB copy.
  The heap total moved less than the footprints because the world tree
  was 37 MiB larger at that moment in the second run.
- Peak resident set over 3,000-frame walks at 8 m/s, alternating before and
  after runs, release build, with other sessions' builds on the machine
  (load averages 27 to 57, `pmset -g therm` CPU speed limit 42 to 100):

  | Run | Peak RSS | Chunks loaded | Footprint hits / computed | Steady frames: mean / p95 / p99 |
  |---|---|---|---|---|
  | Before 1 | 1,693 MiB | 322 | 992 / 498 | 34.6 / 56.8 / 69.6 ms |
  | After 1 | 1,109 MiB | 293 | 548 / 387 | 31.8 / 54.4 / 67.5 ms |
  | Before 2 | 1,269 MiB | 306 | 628 / 388 | 31.9 / 52.6 / 60.5 ms |
  | After 2 | 1,443 MiB | 378 | 816 / 409 | 32.4 / 54.4 / 77.6 ms |

  The load decides how far each run gets (after 2 held 378 chunks against
  306), and with it the resident set, by far more than the 131 MiB the heap
  snapshot shows; these walks cannot resolve the difference. The hit rates
  (1.42 to 2.00 hits per computed footprint after, 1.62 to 1.99 before)
  overlap, as expected with the same 32 footprints.
- `cargo bench -p world_gen --bench terrain` on the floor Mac. The machine
  was shared all evening (load averages 4 to 50) and ran in two modes, a
  run taking either its usual time or about twice it, so the figures are
  alternating runs of the saved before and after binaries, compared within
  one mode:

  | Benchmark | Before | After |
  |---|---|---|
  | `terrain/chunk_columns` (slow mode) | 46.4 to 47.7 ms | 43.3 to 46.1 ms |
  | `terrain_material/material_per_voxel` (slow mode) | 48.6 to 59.6 ms | 48.9 to 49.9 ms |
  | `terrain_tree/chunk_columns_and_build` (fast mode, one pair) | 72.1 ms | 68.7 ms |
  | `terrain_tree/chunk_columns_and_build` (slow mode) | 140.6 to 148.5 ms | 135.9 to 139.4 ms |
  | `terrain_tree/caves_surface_columns_grid_and_build` (slow mode) | 202.2 to 212.4 ms | 207.7 to 210.5 ms |
  | `terrain_tree/chunk_build_only` (fast mode) | 40.5 to 42.2 ms | 43.2 ms |
  | `terrain_tree/chunk_build_only` (slow mode) | 82.0 to 87.3 ms | 84.9 to 86.8 ms |
  | `terrain_tree/caves_underground_build_only` (slow mode) | 130.4 to 133.8 ms | 136.4 to 137.5 ms |

  Generating a footprint costs the same (23.6 against 23.7 ms median when
  timed in one process, interleaved), and reading a column per voxel is no
  slower and faster in most runs (half the bytes to fetch). A footprint's bounds pass is
  four times faster (0.5 against 1.9 ms, timed in one process). Building a
  chunk from columns already generated, as stacked chunks do from the
  cache, is 2 to 5% slower: unpacking costs more than loading an
  eight-byte column in the leaf fill, most of all where caves carve
  (timed in one process against the old source, interleaved: 0 to 5% for
  the heightfield alone, 4 to 7% with caves). This is the cost accepted for
  the memory.
- `ChunkColumns` no longer lends out a slice of `Column`s; code that wants
  one calls `to_vec` (tests do).
- Next: the leaf fill could read the cave range from the footprint's base
  as the plain fill does, and a footprint the cache holds could skip the
  shared pointer's 16-byte header and page rounding; neither is large. The
  bigger items left from decision 0014 are the world tree's pages and the
  brick cache's `Entry`.
