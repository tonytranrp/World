# 0009: How decisions 0006 to 0008 were implemented

Status: accepted, 2026-11-20

## Context

Decisions 0006 (world residency and storage tiers), 0007 (GPU brick cache)
and 0008 (deterministic terrain floats) set the design. Implementing them
needed choices the decisions left open, and one correction: decision 0006
calls a chunk "a level-5 subtree" and a region "level 7", counting levels of
branching; in the tree's own numbering, where a level-L node spans
`4^(L+1)` voxels, a chunk is the subtree under a level-4 node (1024 voxels,
16 m) and a region is the cube under a level-6 node (16384 voxels, 256 m).
Records are append-only, so the corrections and choices are recorded here.

## Decision

### World_Core

- **Chunk storage.** `CHUNK_LEVEL = 4`. Each chunk owns a `Store` (pages and
  free lists) behind an `Arc` in an `Arc`ed chunk table, so a snapshot stays
  one reference count with no allocation (the no-allocation test still
  passes), and the first write to a chunk after a snapshot copies that
  chunk's free lists and table entry only. The children of level-5 nodes are
  chunk ids. A chunk exists only while its region has mixed content.
- **Not resident.** The child word `0xC000_0000` (bits 31 and 30; a real
  uniform word keeps bits 16 to 30 clear). It is allowed only at the chunk
  level and above. Readers report it (`NodeKind::NotResident`, `lookup`
  returning `None`, `is_resident`); edits there fail with
  `EditError::NotResident` and the game layer queues them. The encoding
  gains tag 4 for it in version 1, since no save existed before.
- **Compact bricks.** The byte breakdown decision 0006 asked for showed full
  single-material leaves at 42 to 62% of a generated chunk's live bytes, far
  above the 15% bar. A level-1 block now starts with a three-word header (a
  default mask and a default material, the most common material among its
  full leaves); those leaves have no record, and `Node::info` counts the
  records that remain. Generated chunks shrink 37 to 57% (for example 4.09
  to 2.07 MiB and 8.69 to 5.37 MiB). The save format and the GPU format are
  unchanged, since both are written through the read interface.
- **Building.** `RegionSource` (exact `uniform` answers plus `leaf`) feeds a
  bottom-up builder that writes each leaf and node once. Decoding, terrain,
  compaction, copies, coarse summaries and overlay merges all use it.
  `take_chunk` and `insert_chunk` move a whole chunk between trees in O(1),
  which is how worker threads hand chunks to the main thread.
- **Overlays.** `MaterialId::KEEP` (`u16::MAX`, refused by the material table
  along with id 0) marks "keep the generated base". A region overwritten as
  a whole is stored as its new material even where some voxels did not
  change, because one uniform word is smaller than a mix of KEEP and the
  material.
- **Coarse summaries.** `extract_coarse(level, min, cell_level)` reduces
  every `4^cell_level` cell to its most common solid material (ties to the
  lower id). Saved chunks keep one at the configured coarse cell level.
- **Persistence.** Region files (`r.x.y.z.wvr`, 16³ chunks, zstd-fastest
  records, checksummed table and records, atomic replace) and the edit log
  (`edits.wvl`, checksummed records, torn tail cut on reopen). LZ4
  (`lz4_flex` 0.12, safe modes) for the warm tier. A chunk's primary record
  is its overlay unless the overlay is more than twice the full encoding.

### World_Gen and World_Cpu_Ref

- Terrain v1 is heightfield-only, scalar Perlin of our own, golden hashes
  run on Windows, macOS arm64 and macOS x86_64 under Rosetta.
- **Caves are generator version 2** (now `CURRENT`; version 1 stays
  supported and its hashes unchanged, since its fingerprint ignores the cave
  parameters). Three 3D fBm fields are evaluated every metre, trilinearly
  upsampled to a quarter-metre lattice where
  `f = min(max(|a|, |b|) - tunnel_radius, cavern_threshold - c)` (tunnels
  where two fields cross zero, caverns where a third is high), and `f` is
  trilinearly interpolated to voxels; a solid voxel inside the cave band and
  under `cover_m` of rock is air where `f < 0`. Upsampling the fields before
  combining them costs a sixteenth of the noise of a quarter-metre lattice
  and keeps tunnel walls sharp. Every lattice value depends only on its
  global index and interpolation has a fixed order, so `Generator::material`
  and a chunk's grid agree bit for bit. Interval skipping uses corner bounds
  per cell and a pyramid over them; cubes inside one cell are decided by
  their eight corner voxels (the interpolant is multilinear there), with a
  `1e-3` margin. `Generator::chunk_source` builds a chunk with its caves;
  the coarse tier stays heightfield-only.
- `ChunkSource` proves air, bedrock and soil cubes uniform from a min/max
  pyramid; a surface chunk builds in about 47 ms. `CoarseSource` gives the
  coarse tier one material per cell.
- The CPU reference tracer reports not-resident regions as hits with
  `resident: false`.

### World_Render stage 1 at scale

The first runs with caves showed stage 1 walking every resident record each
frame: about 650,000 records, four random table lookups each, 140 to 220 ms
a frame with nothing to do. The walk is now split. Growth (expand, load)
descends only into nodes that want refining (a child projects to less than
a quarter of its parent, so below a node that does not want refining nothing
does), tests children by squared distance from four slab distances per axis
before any lookup, and is skipped while the camera stays within
`lod.regrow_voxels` of where it last ran and the table has not changed.
Shrink (collapse, evict) resumes across frames under
`lod.max_visits_per_frame`. An idle frame's `run_frame` went from 140 to
220 ms to 4 to 6 ms (development build); a shrink pass within the budget
gives exactly the whole walk's answer (tested). The per-frame eviction check
reads the brick count from the id allocator instead of scanning the table.

### World_Game

- A new `residency` domain owns the world: `StreamedWorld` (plain Rust) with
  the tree, a coarse tree, the warm cache, pending edits, the edit log and a
  thread pool (one I/O thread that alone touches region files, plus
  workers; zero workers runs everything inline for tests). The Bevy plugin
  opens it from `assets/config/residency.ron` and `assets/config/terrain.ron`
  and drives it from the player camera each frame.
- The renderer reads a third tree, the *render tree*: the loaded chunks of
  the world tree (shared through their `Arc`s, never copied) and the coarse
  chunks wherever those are not loaded. Each frame its snapshot goes to
  World_Render's `VoxelWorldFeed` with the edited bricks plus one brick per
  swapped chunk, which makes the GPU cache rebuild that region. Decision
  0007's "coarse levels stay resident" then covers the coarse ring for free.
- An edited chunk that is evicted keeps its content in memory until its
  region file is written, so a quick return never reads a stale file.
- `interaction` digs (left click) and builds (right click) through it, with
  picking by the CPU reference tracer until World_Core's own ray module
  exists.

### World_Core point operations against main

CI's instruction gate (`bench-instructions`, Callgrind, 5% limit against
main) first measured the chunked tree at +73% for random reads, +31 to +77%
for single-voxel writes, +22% for box fills and +27% for an edit after a
snapshot, against 97% fewer instructions for decoding. Callgrind profiles
from the CI artifact, compared function by function, found the causes:

- `step` compiled as a call, so the page reader's cached page went through
  memory at every level. It is now `inline(always)`.
- Every store access inside a chunk ran `Arc::make_mut` on the chunk table
  and the chunk. The writer now checks the chunk out once, at its first
  write, and back in when it leaves.
- A full leaf of several materials was treated like a full single-material
  leaf, so every edit in a dense mixed block repacked its brick. Only
  single-material full leaves are brick defaults; the others patch in place.
- Brick packing made three passes and cleared two arrays; it makes one pass
  and moves the array.
- `set` walked the top levels to check residency before writing; the write
  itself refuses at an unloaded child. Lookups test for not-resident words
  only above the chunk level.

After those fixes reads were still +29% and writes +23%: a lookup crossed
five top levels and then started a second page reader inside the chunk.
The forest now keeps a **chunk index** (chunk key to id, and id to key,
behind `Arc`s that snapshots share and that are copied with the chunk table
on the first write after a snapshot, so snapshots stay O(1)). A point read,
a single-chunk voxel batch or a single-chunk box goes straight to the chunk.
A hit counts only when the id is live and its key matches, so a stale or
missing entry costs a walk from the root and never a wrong answer; chunks
are linked where their position is known (voxel writes, fills, grafts,
region builds) and unlinked when freed, and a write that collapses an
indexed chunk is recorded above it by a graft. Tests compare reads through
the index with cursor walks across moves between trees, unloads, collapses
and snapshots, and the one-million-edit suites pass.

The first write to a chunk after a snapshot copied the chunk's free lists;
they are now shared per size behind `Arc`s and copied only where a write
allocates or frees, so such a write costs reference counts and the page it
touches, as before chunks.

The gate passes (dd8a860), against main: random reads -36%, writes into an
empty tree -2% and into a dense block -4%, batched edits +1%, box fills
+5% (4.6), an edit after a snapshot +3%, encoding 0%, decoding -97%.

Criterion on Windows against main: random reads 2.3 against 6.0 ms per
100k, coherent reads 1.4 against 3.1 ms per 64k, coherent cursor reads
0.45 against 0.45 ms, single-voxel writes 3.8 against 4.3 ms per 10k,
batched edits 1.34 against 1.32 ms, box fills 0.51 against 0.49 ms. A
snapshot is 40 ns instead of 17 (two more reference counts).

## Measured (RTX 4070 laptop, i7-13620H, 2026-11-20)

| What | Before | After |
|---|---|---|
| First edit after a snapshot of a 466 MiB tree | 92.7 µs | 6.2 µs |
| One edit, no snapshot | 0.92 µs | 1.1 µs |
| 1,000 scattered edits after a snapshot | 1.1 ms | 1.08 ms |
| Decoding a save | 1.2 to 1.4 MiB/s | 48 to 62 MiB/s |
| Building one generated 16 m chunk | 490 ms | 39 ms |
| Seed to tree for one chunk (columns and build) | | 73 ms |
| Moving a whole chunk between trees | | 19 µs |
| Compacting a fragmented dense chunk (128³) | | 31 ms |
| LZ4 compress / decompress | | 836 MiB/s / 3.8 GiB/s (1.6x) |
| zstd fastest compress / decompress | | 77 / 347 MiB/s (2.0x) |
| Generated chunk memory | 2.2 to 8.7 MiB | 0.9 to 5.4 MiB |
| Full-detail sphere around the player, debug build | 280 chunks, 298 MiB, 8.0 s | 276 chunks, 151 MiB, 4.8 s |
| GPU trace at 1280x720, full / footprint detail | | 7.8 / 1.47 ms |
| GPU brick encoder, one thread | 103 MiB/s | 326 MiB/s (stored palette blocks copied) |
| Stage 1 frame with nothing to change, 650k records (development build) | 140 to 220 ms | 4 to 6 ms |
| Surface chunk seed to tree, version 1 / version 2 with caves | | 77 / 112 ms |
| Underground chunk with caves (columns and cave grid / build, 97k leaves, 1.4 MiB) | | 40 / 64 ms |
| Full-detail sphere with caves, development build | | 276 chunks, 289 MiB, 9 to 10 s |
| Loading bricks into the GPU pools, 1 / 8 threads | | 153 / 245 MiB/s |

## Consequences

- Snapshots copy nothing, but the first edit to each chunk after a snapshot
  clones that chunk's free lists.
- Terrain changes invalidate saved overlays; the generator version is stored
  in every record, but old generator versions are not yet kept.
- Three trees share chunk data: the world tree (authoritative), the coarse
  tree and the render tree. Only the world tree accepts edits.
- The GPU side is decision 0007 as built in World_Render, with these
  deviations: the representative material above bricks is the voxel-weighted
  mode of the children (so an edit re-evaluates one path); requests carry a
  level and a priority so stage 2 can expand nodes; victims carry their
  block reference so a stale read-back cannot evict a reused id; request
  stamps share the feedback buffer (8 storage buffers, the tier-B minimum);
  the apply pass is four dispatches; the traversal keeps its ancestor stack
  in workgroup memory because D3D12's FXC compiler cannot index a private
  array at runtime; shader constants are mirrored by hand, guarded by a test
  that parses the shader, until Xtask codegen exists.
