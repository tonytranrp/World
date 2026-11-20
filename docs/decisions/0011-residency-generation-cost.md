# 0011: Residency works around the cost of generating chunks

Status: accepted, 2026-11-20

## Context

Decision 0006 regenerates unedited terrain from the seed instead of
storing it, and sets RAM budgets per tier for the floor Mac (2019 MacBook
Pro, i7-9750H). The first walks on that machine showed residency bound by
generation, not by I/O or ordering: 6 to 8 chunks a second while the game
renders, so the full-detail sphere was about half empty walking at 4 m/s.
Per 16 m chunk:

- every chunk computed its footprint's whole 1024 x 1024 heightfield
  (24 ms), again for each chunk stacked in the same column, though columns
  depend only on x and z;
- an all-air chunk cost about as much as a surface chunk (36 ms);
- the build itself took 50 ms without caves and 85 ms with them, mostly
  leaf encoding and per-voxel cave evaluation.

## Decision

- **Workers share a bounded cache of column footprints**, keyed by the
  footprint's x and z and the generator, least recently used out, ties to
  the lower coordinate. Concurrent requests for one footprint compute it
  once. Its budget is `column_cache_mib` in `residency.ron` (256 MiB, 32
  footprints at 8 MiB), a new RAM item on top of decision 0006's tiers; 0
  turns it off. Overlay merges and saves take their base from it too.
- **Chunks proven uniform from the footprint's bounds are not built**: air
  at or above every column's top (caves carve only solid voxels), bedrock
  below every column's bedrock where no cave can reach. World_Gen states
  the proof (`Generator::uniform_chunk`) and tests it against the full
  build.
- **Buried chunks load later**: a chunk entirely below its footprint's
  lowest column counts as `buried_delay_m` (32 m) farther away. The ground
  under the player still comes first, and every chunk of the full-detail
  sphere still comes before any lookahead chunk.
- **The builder asks a source for a level-1 node's 64 leaves at once**
  (`RegionSource::brick`, defaulting to leaf by leaf), so terrain sources
  can share work across a brick. Faster leaf encoding, row-at-a-time
  columns and per-axis cave upsampling changed no generated voxel: the
  World_Gen golden hashes are unchanged and the fast paths are tested
  against the per-voxel answers.

## Alternatives considered

- **Storing generated chunks on disk.** It would make return visits cheap
  but contradicts decision 0006 (unedited terrain is never stored) and
  costs SSD space and writes; the warm tier keeps recent ones in RAM
  instead.
- **Generating whole columns as one job.** Shares the heightfield without a
  cache, but a column's buried chunks would delay its surface chunk behind
  them, the opposite of the ordering above.
- **Explicit SIMD for the relief noise** (about half the column time).
  Decision 0008 forbids runtime SIMD dispatch for bit-exact terrain;
  compile-time SIMD would need per-target golden checks. Not now.

## Consequences

- On the floor Mac, `cargo bench -p world_gen --bench terrain`: columns 24.6
  to 15.6 ms, a surface chunk with caves 130 to 62 ms, an underground one
  122 to 58 ms. Stacked chunks skip the heightfield and air chunks take
  microseconds.
- Walking at 4 m/s in mac-floor, before the World_Gen speedups: holes in the
  full-detail sphere 90 to 98 per frame on average before, 63 to 73 after.
- RAM on the floor Mac grows by up to `column_cache_mib`; lowering it lowers
  the hit rate (about 1.35 requests per computed footprint while walking).
- Buried chunks, mostly cave carving, remain the expensive ones; under fast
  motion the sphere still falls behind.
