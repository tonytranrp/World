# GPU voxel cache: residency, layout and limits

Written 2026-11-19 from a research pass (web sources plus our decisions 0004 and
0005 and the engine plan). It informs decision 0007. Confidence marks: **measured
(third party)** means a source reports a number; **unverified** means a single
source or our own reasoning; nothing here was measured by us.

## The question

The world is a 16.384 km cube of 1/64 m voxels, ray traced and never meshed. The
CPU 64-tree is authoritative. How should the GPU hold only what rays need, in a
fixed budget (about 1 GB on the 2019 MacBook Pro's 4 GB Radeon Pro 5300M, 2 GB on
the 8 GB RTX 4070 Laptop), laid out for GPU caches, and degrade gracefully when
the budget is full?

## Sources and what they teach

### voxelbee devlogs #3 to #6 (YouTube, 2021)

- Devlog #5 (https://youtu.be/i7vq-HY10hI): every octree "voxel" is an entry in
  one flat array. Rendered entries are moved toward the front, unrendered ones
  drift back and are overwritten by new loads. Everything runs in compute
  shaders. Tested at 14 MB and 1 MB with over a trillion voxels in view and 64 GB
  addressable. He claims entries used together end up adjacent. No timings, no
  code, no written follow-up (his GitHub has no engine repo). **Unverified.**
- Devlog #4 (https://youtu.be/TPg_LwWM0Bo): his first cache sorted 10 to 20
  million page entries per frame, was too slow, and picked pages still in use.
- Devlog #6 (https://youtu.be/BThCGgdYvVE): load priority is voxel size over
  distance, so large near voxels arrive first.
- Our assessment: moving data costs bandwidth (bricks of about 1 KB make it worse
  than his small nodes), needs atomics to elect one mover when many rays touch an
  entry, breaks every parent pointer to a moved entry, and splits contiguous
  child groups. The locality claim is plausible but unmeasured. **Keep the idea
  of VRAM as a demand-paged working set with graceful coarsening; drop the
  move-to-front mechanism.**

### GigaVoxels (Crassin et al. 2009, thesis 2011) and its successors

- Node pool plus brick pool; each node points at a contiguous tile of its
  children. Bricks were 16³ or 32³, mipmapped.
  ([I3D 2009](http://gigavoxels.inria.fr/Publications/2009/CNLE09/CNLE09.pdf),
  [CNE09](https://www.icare3d.org/research/publications/CNE09/IntelConf_Final.pdf),
  [SIGGRAPH 2009 slides](http://gigavoxels.imag.fr/Publications/2009/CNLSE09/GigaVoxels_Siggraph09_Slides.pdf))
- GPU LRU without moving data: every element has a usage stamp that rays set; a
  pass masks this frame's users; two stream compactions split a list of element
  addresses into used and unused, which are concatenated so the oldest come
  first. New data overwrites the oldest slots. Coarsening is implicit: an evicted
  tile's subtree stops being stamped and ages out.
- Sarton et al., TVCG 2019 (64³ bricks, multi-level page table on the GPU):
  cache management under 2 ms per frame, brick loads 2.1 to 13.6 ms.
  **Measured (third party).**
  ([paper](https://josarton.netlify.app/files/papers/TVCG2019.pdf))
- Hadwiger et al. 2012: per-screen-tile GPU hash tables collect misses; full
  rows drop requests until a later frame.
  ([paper](https://vccvisualization.org/publications/2012_hadwiger_petascale.pdf))
- GigaVoxels DP (2024) launches producers from rays with CUDA dynamic
  parallelism, which wgpu does not have. ([HAL](https://hal.science/hal-04654692))
- The thesis chapters on the cache could not be text-extracted; details come from
  the papers and slides. **Thesis-only details unverified.**

### Virtual texturing and Nanite

- id Tech 5 software virtual textures: a fully resident physical page pool, a
  feedback pass, and lookups that fall back to the finest resident (blurrier)
  ancestor without stalling.
  ([van Waveren 2012](https://mrelusive.com/publications/papers/Software-Virtual-Textures.pdf))
- Nebula deduplicates requests in one pass with `atomicExchange` on a status
  word plus `atomicAdd` on a list counter.
  ([Nebula](https://nebulatrifid.org/?p=1493)) Using `atomicMax(stamp, frame)`
  instead removes the per-frame clear (our variant, **unverified** in
  production).
- Nanite keeps root pages resident, the GPU requests missing clusters, the CPU
  streams by priority, and coarser resident data stands in meanwhile.
  ([Karis 2021](https://www.advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf))
- Hardware sparse resources: wgpu has none through version 30
  ([discussion #6111](https://github.com/gfx-rs/wgpu/discussions/6111)); Metal
  sparse heaps cover textures only. Not an option for us.

### Recent voxel engines and papers

- **dubiousconst282 64-tree guide and VoxelRT**: nodes `{IsLeaf:1, ChildPtr:31,
  ChildMask:64}` with children packed contiguously, child at `ChildPtr +
  popcount(mask & below(i))`, so one dependent load per level. Ancestor
  memoisation about 1.9x, single-cell-skip coalescing 21%, octant mirroring 10%;
  0.62 B/voxel vs ESVO's 1.02 on Bistro. Tree64 led primary rays in 5 of 6
  scenes. **Measured (third party).**
  ([guide](https://dubiousconst282.github.io/2024/10/03/voxel-ray-tracing/),
  [VoxelRT](https://github.com/dubiousconst282/VoxelRT))
- **Aokana (2025)**: 256³ chunks as shallow SVDAGs, a CPU implicit octree
  chooses chunks by size and distance, about 5% of the scene resident, 292 to
  424 MB VRAM in tests. Distance-driven, not ray-guided.
  ([arXiv 2505.02017](https://arxiv.org/html/2505.02017))
- **Kiln** (web volume renderer), the best documented ray-guided streamer found:
  desired set by screen-space error (8 px) with hysteresis (0.7x to 1x, stay
  split only if children are resident); rays set one bit per cell entered, read
  back every 4 frames; the coarsest level pinned; LRU eviction skips anything
  used in the last 30 frames; evicted cells point back to their ancestor.
  ([streaming](https://kilnrender.com/architecture/streaming.html))
- **lardratboy streamer** (65,536³ world): "nothing waits", one LOD step
  between neighbours, and the "any" coarse-occupancy rule bloats porous content
  1.2x to 6.8x. **Single source.**
  ([explainer](https://lardratboy.github.io/testbed/streamer/renderer-explainer.html))
- **Voxolith** (WebGPU): 8³ bricks where 98% hold 8 or fewer materials; all
  index tables in one buffer to fit binding limits; GPU-side pool copies take
  1 ms or less vs 150 to 270 ms re-uploading from the CPU.
  ([storage](https://voxolith.github.io/docs/renderer/storage/))
- **Douglas Dwyer (Octo)**: coarse LODs generated directly by worldgen; an edit
  marks enclosing octants dirty and only those are re-downsampled; LODs cached
  on disk. ([devlog #16](https://youtu.be/74M-IxtSVMg))
- **AMD Brixelizer**: 8³ distance-field bricks in a shared atlas, toroidal
  cascade updates. Relevant later for GI, not geometry.
  ([GDC 2023](https://gpuopen.com/download/GDC-2023-Sparse-Distance-Fields-For-Games.pdf))
- **ESVO** stops descending when a voxel is smaller than the ray's footprint.
  ([tech report](https://www.nvidia.com/docs/IO/88972/nvr-2010-001.pdf))
- HashDAG, GPU SVDAG editing, transform-aware SVDAGs and occupancy-in-pointer
  encodings all trade edit cost or format fit for compression; they confirm
  decision 0005's rejection of DAGs for a destructible world.
- Write cost on Apple GPUs: staggering cache-stamp writes cut atomic bytes
  written from 617 MiB to 19 MiB per frame and a pass from 6 to 2 ms on an M2.
  **Single source.** ([nelari.us](https://nelari.us/post/voxel-ray-tracing/))
- Teardown, Atomontage, John Lin and Ethan Gore publish no cache internals we
  could find.

## GPU memory facts

| | RTX 4070 Laptop (Ada) | Radeon Pro 5300M (RDNA1) | Apple M-series |
|---|---|---|---|
| SIMD width | 32 | 32 or 64 (32 reported under Metal, single source) | 32 |
| Small cache | 128 KB L1/shared per SM | 16 KB L0 per CU | about 8 KB data cache per core |
| Line size | 128 B in four 32 B sectors | 128 B | 128 B |
| Last level | 32 MB L2 | 2 MB L2 | M1: 768 KB L2 + 8 MB SLC |
| Bandwidth | about 256 GB/s | 192 GB/s | M5: 153 GB/s |

Sources: NVIDIA Ada tuning guide, RDNA whitepaper, philipturner/metal-benchmarks,
vendor spec pages (via search synthesis; see the decision for the list).

What follows from it:

- Scattered 4 B reads cost a full 32 B sector each; neighbouring siblings in one
  128 B line are nearly free.
- 16 B nodes aligned to 16 B never straddle a sector and fit 8 per line. The
  guide's 12 B node saves 25% but can straddle; with tiny caches on the floor
  Mac and Apple GPUs, a 16 B node with a spare word for LOD data is the better
  trade (**our judgement**).
- Keep the per-ray hot set small: node records and leaf masks first, palettes
  and indices last, read only at the final hit.
- Storage buffers, not textures, suit a sparse tree visited one record at a
  time.

## Pixel footprint

At 720p with a 70° vertical field of view a pixel spans about 1.7 mrad. One
1.56 cm voxel becomes sub-pixel at about 9 m, a 6.25 cm leaf cell at about 37 m,
a 25 cm brick cell at about 147 m, a 1 m level-2 cell at about 590 m. At 1440p
the distances double. (**Our calculation.**) The 64 m full-detail radius is
therefore what the CPU and physics need; the GPU needs per-voxel payloads only
within about 10 to 20 m, leaf masks out to about 64 m, and node masks beyond.

## wgpu 30 limits that matter

- Storage binding size is clamped to i32::MAX on DX12 and Vulkan because of
  driver bugs; a maintainer says Metal lacks the problem and full 4 GB buffers
  have worked there. Query limits at runtime instead of assuming 256 MiB.
  ([#8105](https://github.com/gfx-rs/wgpu/issues/8105))
- Binding arrays of buffers are Vulkan-only, so pool segments are separate
  bindings chosen by a `switch`; fewer, larger segments are better.
- Metal shares one argument table: wgpu 30 caps buffers per stage at 29.
- 32-bit atomics are everywhere. 64-bit atomics are min/max-only on Metal and
  failed on several Apple and AMD GPUs in one test; assume none on the floor Mac.
  ([AtomicsWorkaround](https://github.com/philipturner/ue5-nanite-macos/blob/main/AtomicsWorkaround/README.md))
- Subgroups exist on all three backends, but naga rejects `enable subgroups;`
  ([#8202](https://github.com/gfx-rs/wgpu/issues/8202)); optional only.
- `StagingBelt` beats `queue.write_buffer`, which allocates often on Metal
  ([iced #2357](https://github.com/iced-rs/iced/pull/2357)); wgpu 30 added
  `finish_and_recall_on_submit`, 16-bit integers in WGSL and procedural AABB
  geometry in BLASes. ([v30.0.0](https://github.com/gfx-rs/wgpu/releases/tag/v30.0.0))
- Bevy's `Readback` reads a fixed buffer range asynchronously; expect 2 to 3
  frames of latency (**unverified**).

## Crates

- `offset-allocator` 0.2.0: safe Rust, O(1), at most 12.5% waste, in Bevy's
  lockfile, no release since 2024. Fits variable-size brick blocks.
- `range-alloc` 0.1.5 (wgpu maintainers, 2026): best-fit fallback.
- `bytemuck` (already used) for raw pools; `encase` only for uniform blocks.

## Open, to verify on hardware

The 5300M's real `max_storage_buffer_binding_size`, `maxBufferLength`,
subgroup size and 64-bit atomic support; Bevy 0.20 render-graph details;
measured readback latency.
