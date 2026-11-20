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

- Terrain v1 is heightfield-only (no caves yet), scalar Perlin of our own,
  golden hashes run on Windows, macOS arm64 and macOS x86_64 under Rosetta.
- `ChunkSource` proves air, bedrock and soil cubes uniform from a min/max
  pyramid; a surface chunk builds in about 47 ms. `CoarseSource` gives the
  coarse tier one material per cell.
- The CPU reference tracer reports not-resident regions as hits with
  `resident: false`.

### World_Game

- A new `residency` domain owns the world: `StreamedWorld` (plain Rust) with
  the tree, a coarse tree, the warm cache, pending edits, the edit log and a
  thread pool (one I/O thread that alone touches region files, plus
  workers; zero workers runs everything inline for tests). The Bevy plugin
  opens it from `assets/config/residency.ron` and `assets/config/terrain.ron`
  and drives it from the player camera each frame.
- An edited chunk that is evicted keeps its content in memory until its
  region file is written, so a quick return never reads a stale file.
- `interaction` digs (left click) and builds (right click) through it, with
  picking by the CPU reference tracer until World_Core's own ray module
  exists.

## Consequences

- Snapshots copy nothing, but the first edit to each chunk after a snapshot
  clones that chunk's free lists.
- Terrain changes invalidate saved overlays; the generator version is stored
  in every record, but old generator versions are not yet kept.
- The coarse tier is a second tree the renderer must read beyond the
  full-detail radius.
