# CPU voxel world tiers, edits as diffs, and the upload path

Written 2026-11-19 from a research pass (web sources plus our decisions 0004 and
0005, the engine plan and the World_Core tree and serialize code). It informs
decisions 0006 and 0008. Confidence marks: **measured (third party)**,
**measured by us** (our benchmarks from 2026-11-19), **estimate** (our
reasoning, to be benchmarked).

## The question

At 1/64 m there are about 4,096 surface voxels per m². Our tree holds about
7.5 B per surface voxel on terrain (**measured by us**, 5.5 B live; free/total
words 0.48 after building, 0.88 after a million random edits), so about 30 KB
per m²: 1.6 GB for a 128 m radius, 30 GB per km². The 16 km world can never
live in RAM. How do engines keep worlds larger than RAM, what should be stored
versus regenerated, and how should data reach the GPU?

## How others do it

- **Regenerate unedited terrain; store only edits.** Space Engineers keeps no
  procedural voxels in memory unless altered and generates distant asteroids at
  coarse detail only
  ([Marek Rosa 2014](https://blog.marekrosa.org/2014/12/space-engineers-super-large-worlds_17/)).
  Luanti, Voxel Tools, bevy_voxel_world and Veloren's persistence store edited
  units only. A small MMO server (Bestia) measured generation at about 20 ms per
  chunk against 0.2 ms RLE decode: decoding a stored chunk is about 100x cheaper
  than generating it ([docs](https://docs.bestia-game.net/docs/server/chunk-caching/)).
  **Single source.**
- **Never cap or forget edits.** No Man's Sky's 15,000-edit limit and terrain
  regrowth are the cautionary tale
  ([guide](https://steamcommunity.com/sharedfiles/filedetails/?id=2526352095)).
- **Compressed in-RAM tier.** Vintage Story keeps chunks compressed in memory
  and protects recently used ones
  ([API](https://apidocs.vintagestory.at/api/Vintagestory.API.Common.IWorldChunk.html)).
- **Region files.** Minecraft Anvil puts 32×32 chunks per file behind a sector
  table ([wiki](https://minecraft.wiki/w/Region_file_format)); Paper's
  SectorFile added xxHash checks and zstd/LZ4 for 1.3x to 1.6x smaller worlds
  ([PR #10231](https://github.com/PaperMC/Paper/pull/10231)).
- **Distant Horizons** (Minecraft LOD) stores column runs per detail level in
  SQLite; switching zstd stream to block compression made reads 6x and writes
  3x faster ([2.4.0 changelog](https://modrinth.com/mod/distanthorizons/version/2.4.0-b-1.21.10)).
  **Measured (third party).**
- **OpenVDB removed its out-of-core delayed loading and `io::MappedFile` in
  14.0** (2026-10-29) ([release notes](https://www.openvdb.org/documentation/doxygen/changes.html)).
  NanoVDB builds one contiguous, defragmented block by counting nodes per level,
  allocating exactly, then filling ([FAQ](https://www.openvdb.org/documentation/doxygen/NanoVDB_FAQ.html)).
- **Aokana** pre-stores everything and loads by distance, about 5% resident
  ([arXiv](https://arxiv.org/html/2505.02017)). **Teardown** saves the whole
  world with RLE to 15 to 20% of raw
  ([quicksave](https://blog.voxagon.se/2020/11/18/teardown-quicksave.html)).
- **HashDAG** garbage collection took 25 s unoptimised at 64k³
  ([paper](https://graphics.tudelft.nl/Publications-new/2020/CBE20/ModifyingCompressedVoxels-main.pdf)),
  which again rules out a CPU DAG for a destructible world.
- John Lin, Ethan Gore, Atomontage and Voxel Farm publish no residency or
  compression numbers we could find.

## Budget arithmetic for our decisions

(**Estimates**, to be checked with real views.)

- The 64 m full-detail disc: π·64² m² × 4,096 ≈ 53 M surface voxels, about
  0.4 GB at 7.5 B, 0.8 GB with a 2x allowance for caves and props. With an
  eviction radius of 96 m for hysteresis: about 0.9 GB.
- Coarse detail must follow pixel footprint, not doubling distance rings:
  constant-ratio rings would cost about 0.75x the core per ring (about 2.5 GB
  for seven). Footprint-driven coarse bricks generated on demand fit in a fixed
  0.5 GB LRU.

## Regeneration cost and determinism

- Noise throughput per core: FastNoiseLite 3D Perlin 47.9 M/s, FastNoise2 AVX2
  261 to 268 M/s (3D) and 466 to 624 M/s (2D)
  ([FastNoiseLite](https://github.com/Auburn/FastNoiseLite)). **Measured (third
  party).** A 16 m chunk has about 1.05 M columns: 6 octaves of 2D fBm cost
  about 13 ms with SIMD-class noise or about 90 ms scalar (**estimate**). Full
  3D noise over 1024³ is about 4 s even with FastNoise2, so caves need a coarse
  lattice with interpolation (64x fewer samples) and skipping of subtrees with
  interval bounds, as Octo and Baert do
  ([Baert 2013](https://www.forceflow.be/2013/03/20/out-of-core-sparse-voxel-octree-building-skipping-empty-space/)).
- Today building dominates: 786k column fills take 4.6 s (**measured by us**),
  about 6 s per chunk. A bottom-up builder should reach 10 to 30 ms
  (**estimate**).
- Determinism is load-bearing once edits are diffs: SIMD noise libraries pick
  instruction sets at runtime and AVX2 paths use FMA (simdnoise), FastNoise2
  dispatches SSE2 to AVX-512 or NEON at runtime, and platform libm differs.
  Plain f32 without FMA (Rust does not contract `a*b+c` by default) and without
  libm transcendentals, checked by golden hashes on x86 and ARM, is the safe
  path. Version the generator: changing a parameter changes every value
  ([bairn docs](https://docs.rs/bairn/latest/bairn/noise/index.html)).

## Storing edits

Options considered: (a) an overlay 64-tree with a reserved KEEP material so
untouched subtrees collapse to one uniform KEEP child, reusing our codec and
canonical form; (b) a journal of CSG operations (small but replay-dependent);
(c) the full subtree of each edited chunk. Recommended: (a), falling back to
(c) when the overlay exceeds about half the full chunk, with (b) only as a
write-ahead log baked into (a). A 2 m crater adds roughly 100k boundary voxels,
about 20 to 40 KB compressed (**estimate**). Edited chunks also store coarse
mip summaries so destruction stays visible far away.

## Memory efficiency and fragmentation

- Per-chunk arenas built in depth-first order (NanoVDB's count, allocate, fill)
  give exact block sizes and contiguous children for free, make eviction a drop,
  and let compaction rebuild one chunk at a time on a worker, swapped in if the
  chunk's generation counter has not changed.
- Fully solid leaves inside surface bricks each cost a 12 B record today. A
  "full" mask per brick could save 10 to 30% (**estimate**, profile first with a
  bytes breakdown in `stats()`). Veloren's "most common block as default" gave
  7x less chunk memory ([devlog 87](https://veloren.net/blog/devblog-87/)).
- TLSF-style allocators (offset-allocator) bound waste at 12.5% but gain little
  for our small, page-bound blocks.

## Decode and build

Our decoder rebuilds one voxel at a time: 1.2 to 1.4 MiB/s, about 100x slower
than encode (**measured by us**), so a chunk takes seconds to load. The encoded
body is pre-order and canonical, so a recursive reader can write record and
palette blocks directly, parents after children, and keep the validator.

## Compression and disk

- `lz4_flex` (safe by default, 2.3 to 4.5 GiB/s decode in its benchmarks) for the
  warm RAM tier. Pin at least 0.12.1: RUSTSEC-2026-0041 leaked buffer contents
  in older decoders ([advisory](https://rustsec.org/advisories/RUSTSEC-2026-0041)).
- `ruzstd` (pure Rust, already in the lockfile through Bevy) for region blobs at
  its fastest level. Its docs conflict on whether higher levels are implemented;
  check before relying on them.
- `memmap2` file maps are `unsafe` and undefined behaviour if the file changes;
  our crates deny `unsafe`. Use positioned reads on IO threads. Embedded
  databases: `redb` changed major format versions twice in 2026, a risk for
  long-lived saves. A small region format with an xxh3-checked sector table keeps
  the format ours.

## Upload path

- wgpu 30 has no transfer queue; the issue was closed as not planned and
  multi-queue is an unsafe native-only RFC
  ([#5576](https://github.com/gfx-rs/wgpu/issues/5576),
  [#8844](https://github.com/gfx-rs/wgpu/issues/8844)).
- `queue.write_buffer` allocates a staging buffer per call; use `StagingBelt`.
  Since wgpu 29 mapped write views are `WriteOnly<[u8]>`, written with
  `copy_from_slice`.
- PCIe 3 x16 reaches about 11 to 12 GB/s for large transfers but only 1.2 to
  2.9 GB/s for 16 KB ones ([NVIDIA forums](https://forums.developer.nvidia.com/t/the-change-of-speed-when-copying-data-between-host-and-device/176003)):
  batch bricks into a few large copies, then scatter them on the GPU, as Bevy's
  sparse-upload PR does ([#23242](https://github.com/bevyengine/bevy/pull/23242)).
- Apple unified memory makes small copies cheap, but wgpu buffers are either
  mapped or GPU-usable, never both, so zero-copy into a live pool is not
  expressible safely; keep the belt.
- Readback: a ring of 3 to 4 buffers, never block the render thread.
- `ExtractResource` clones an `Arc` snapshot in O(1), but pipelined rendering
  keeps two snapshots alive, so the first write to each shared page per frame
  copies it: the source of our 92.7 µs first-edit cost on a 517 MiB tree
  (**measured by us**). Per-chunk directories shrink it.

## Change tracking and threads

- Accumulate the dirty bricks of every `apply` per frame, give each chunk a
  generation counter, upload edited resident bricks first and invalidate them if
  the budget runs out, so a stale brick is never shown.
- One writer owns the tree; workers build standalone chunk arenas (generate,
  decode, compact) and send them over `crossbeam-channel`; the main thread
  grafts them. Bevy's AsyncCompute pool gets 25% of cores by default, so size a
  dedicated pool or raise that share.
- Edits to chunks that are not resident go to that chunk's log and are applied
  after it loads.

## Crates

Trusted: `rayon`, `crossbeam-channel`, `arc-swap`, `lz4_flex` (0.12.1 or newer),
`ruzstd` (decode and fastest encode). Avoid: `simdnoise` (2020, FMA),
`memmap2` (our `unsafe` rule), `redb` for saves (format churn). Reference only:
`voxelis`, `tree64`, `quick-noise` (too new).
