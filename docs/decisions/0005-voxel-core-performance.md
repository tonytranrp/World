# 0005: Voxel core performance and tooling

Status: accepted, 2026-11-19. Partly superseded: decision 0006 adopts per-chunk
storage (the rejected top-level grid, for storage only), and decision 0007
replaces zero-copy page upload with a GPU format and the fixed 256 MiB segments
with runtime limits.

## Context

Decision 0004 built the voxel core: a 10-level 64-tree in copy-on-write
pages, canonical, with snapshots and a binary format. Before the CPU tracer,
the GPU tracer and the brick cache make its layout a format other code
depends on, two research passes compared it against published work (the
dubiousconst282 64-tree guide and VoxelRT, ESVO, SVDAG, SSVDAG, HashDAG,
Aokana, GigaVoxels, OpenVDB) and against the crates that could replace parts
of it. The rule was to adopt reliable existing work rather than write our
own, and to check reliability first: maintenance in 2025-2026, users,
license, `unsafe`, and compatibility with Rust 1.97.1, Bevy 0.20.1 and
wgpu 30.0.2.

No Rust voxel crate passed: tree64 (about 200 downloads, one maintainer, a
different node format, no traversal), VoxelHex (unclear license), voxelis (a
DAG for static data), and the meshing crates, which our rules forbid. They
stay references. What we adopt are techniques and general-purpose crates.

## Decision

### Adopted now

- **Leaves live inline in their level-1 parent.** A level-1 branch's block
  holds a fixed three-word record `[mask_lo, mask_hi, payload]` per non-air
  child, at `base + 3 * rank`. A payload with bit 31 set is one material;
  otherwise it is the offset of a palette block: a header `p | b << 8`, the
  palette (two u16 per word, ascending) and the indices LSB-first at the
  smallest width `b` of 1, 2, 4 or 8. Leaf nodes no longer exist; every node
  is a branch of level 1 to 9 and `Node::info` is reserved (zero). A
  single-material surface leaf drops from 24 to 12 bytes, a two-material
  leaf with 32 solid voxels from 28 to 24 (HashDAG stores its last two levels
  inline for the same reason). The on-disk format is unchanged: it is written
  through the read interface, and the golden bytes still match.
- **A two-level page directory.** The first write after a snapshot copies
  the directory and the page chunks it touches, not one `Arc` per page of the
  whole tree.
- **`bytemuck::Pod` on `Node`**, so node and word pages upload to the GPU
  without a copy. The `unsafe` stays inside bytemuck.
- **Fast reads and a cursor.** `get` reads node and word values directly. A
  `TreeCursor` caches the path of the last lookup and restarts from the
  highest level where the next coordinate differs (OpenVDB's
  `ValueAccessor`).
- **One descent per edit.** A write returns the previous material, so `apply`
  no longer reads every voxel twice, and a batch runs in tree-path order for
  locality.
- **One read interface for every voxel source.** The world tree, its
  snapshots and a new `DenseGrid` for dynamic objects implement the same
  generic traits, so the CPU tracer, physics queries and AO are written once
  and compiled per source (Veloren's `ReadVol` works the same way).
- **`target-cpu=x86-64-v2`** on the x86_64 Windows and macOS targets. On the
  baseline target `u64::count_ones` is about 15 instructions instead of
  POPCNT, and the tree counts bits at every level. Windows 11 24H2 already
  requires POPCNT, and the 2019 MacBook Pro has it.
- **Tooling:**
  - Criterion stays for wall time on our machines.
  - gungraun counts instructions on a Linux CI job and fails on a
    regression. Instruction counts vary about 0.1% on GitHub runners, while
    wall time varies up to 3x.
  - cargo-fuzz fuzzes the decoder weekly.
  - A dhat test proves reads and snapshots allocate nothing.
  - cargo-mutants measures how many code changes the tests catch, weekly.
- **Hashing policy.** Hot lookups use `rustc-hash` (or `bevy_platform` maps
  in Engine crates). Anything iterated for output, generation or saves uses a
  `BTreeMap`, a sorted `Vec` or `indexmap`, because hashbrown's iteration
  order differs between x86 and ARM.

### Adopted at a later step

- **CPU reference tracer:**
  - The dubiousconst282 traversal: mask skipping, an ancestor stack and
    octant mirroring. It ran 1.6 to 1.9 times faster than ESVO in its
    author's benchmark.
  - The choices that decide a hit are integer comparisons. WGSL does not
    promise reproducible floats, and wgpu always compiles Metal shaders with
    fast math. The GPU therefore returns integer hits (voxel, face,
    material), and the distance is compared with a stated tolerance. A mip
    level is read as "stop at level L; a set mask bit is solid".
- **Procedural terrain:** bottom-up subtree construction from generator
  output, grafted in with one write. Saves wrap the body in a zstd frame
  (`ruzstd` at its fastest level, already in the lockfile through Bevy).
- **Brick cache:**
  - The GPU pool is split into segments of at most 256 MiB. One binding is
    capped at 2^31 - 1 bytes on DX12 and Vulkan, and Metal guarantees only
    256 MB. The plan's "up to 2 GB" pool is the sum of those segments.
  - `offset-allocator` places bricks in each segment. It is safe Rust and
    already in the lockfile through Bevy.
  - Uploads use `wgpu::util::StagingBelt` within the per-frame budget.
  - Bevy's `ExtractResource` hands the frame's snapshot to the render world.
  - `Readback` returns the feedback buffer.
  - Page compaction and release, so a long walk does not only grow memory.
- **GPU primary-ray pass:** Tracy spans in World_Core behind a feature.

### Rejected

- **A DAG or HashDAG for the CPU tree.** Every destruction edit would copy,
  rehash and later garbage-collect shared subtrees, and HashDAG traverses 1.5
  to 2 times slower. The ray-guided cache and regeneration from the seed
  already keep the whole world out of VRAM.
- **A top-level grid of shallow trees** (recommended by the guide and by
  Aokana). Our top four levels are already a sparse grid of at most a few
  nodes, a cursor skips them for nearby lookups, and a chunk is a level-5
  subtree that can be generated, saved, encoded or freed on its own. A
  separate grid would change the format for a small gain.
- **8x8x8 leaves.** Thin surfaces would cost more: a 64-byte mask per
  about 64 surface voxels, against 8 bytes per 16.
- **These crates:**
  - `left-right`: keeps two copies of the tree.
  - `im`: unmaintained.
  - `imbl`: MPL-2.0.
  - `slotmap`: 64-bit keys.
  - `generational-arena`: no release since 2023.
  - `triomphe`: a marginal gain.
  - `rkyv`: its format is tied to Rust layouts.
  - `bitpacking`: works on SIMD blocks of 32 or more.
  - `gpu-allocator`: wgpu does not expose it.
  - `ahash`: randomly seeded.
  - `divan`: no saved baselines.
  - `puffin`: overlaps with Tracy.
- **Prefetching and portable SIMD.** Software prefetch needs `unsafe`,
  `std::simd` is still nightly-only, and a single branchy ray gains nothing
  from SIMD.

## Consequences

- The node layout in `tree/node` changes before any shader mirrors it.
  Decision 0004's leaf section is superseded by the inline record above.
- `World_Testkit/src/dense/ray.rs` promises bit-identical distances. That
  holds CPU against CPU only. GPU comparisons must use integer hits and a
  distance tolerance, and the WGSL mirror cannot use infinities.
- The layout constants the GPU will mirror (`UNIFORM_BIT`, `RECORD_WORDS`,
  the page lengths) are exported from `world_core::tree`. Codegen reads them
  from there when the shader mirror is written.
- CI gains three jobs that are not required checks: instruction-count
  benchmarks on Linux for every PR, and weekly decoder fuzzing and mutation
  testing.
