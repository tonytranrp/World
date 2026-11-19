# 0006: World residency and storage tiers

Status: accepted, 2026-11-19

## Context

At 1/64 m voxels a surface costs about 4,096 voxels per m², and the tree holds
about 7.5 bytes per surface voxel on terrain. That is about 30 KB per m², so the
16.384 km world cannot live in RAM, and decision 0004 assumed it could. The
owner set the requirements: full detail within 64 m of the player, every edit
kept forever anywhere on the map, about 4 GB of voxel RAM on the 16 GB 2019
MacBook Pro, and an SSD tier allowed during play. The research is in
`../research/cpu-voxel-world-tiers.md`.

## Decision

### The chunk is the unit of residency

A chunk is a level-5 subtree, 1024³ voxels or 16 m per side. It is the unit that
is generated, loaded, saved, compressed and freed. Each resident chunk owns its
own storage (node and word pages, free lists); levels 6 to 9 are a small shared
top. This amends decision 0005's rejection of a top-level grid for storage only:
the logical tree, the read interface and the serialization format are unchanged.

### The tree marks what is not loaded

A child can be not resident, distinct from air. Readers, the cursor, the tracer
traits and edits report it rather than treating it as empty.

### Tiers

| Tier | Holds | Budget on the floor Mac |
|---|---|---|
| Hot | Every chunk within 64 m, evicted beyond 96 m (hysteresis), merged with its edits | 1.3 GB cap |
| Coarse | Coarse bricks for distant detail, generated on demand or from stored summaries, least recently used first | 0.5 GB |
| Warm | LZ4-compressed chunks recently evicted or prefetched, edited chunks weighted higher | 0.4 GB |
| SSD | Region files and a write-ahead log | unbounded |

With snapshot copy slack, worker scratch, staging and headroom the total is about
3.6 GB. The PC may double the coarse and warm tiers; on Apple unified memory the
GPU pool comes out of the same RAM.

### What is stored

- Unedited terrain is never stored. It is regenerated from the seed (decision
  0008 makes that exact).
- An edited chunk is stored as an overlay tree against its regenerated base,
  using a reserved KEEP material so untouched parts collapse; when the overlay
  exceeds about half of the full chunk, the full subtree is stored instead. Its
  coarse summaries (levels 2 to 4) are stored with it so destruction is visible
  from afar.
- Every edit batch is appended to the write-ahead log before the frame ends.
  Region files group the edited chunks of a 256 m region (level 7) behind a
  sector table with xxh3 checksums, and are rewritten from the log on eviction,
  save and quit, or when the log grows large.
- Edits to a chunk that is not resident are logged and applied when it loads.
  Nothing is ever capped or forgotten.
- Disk access uses positioned reads on IO threads, not memory maps.

### Threads and change tracking

One writer owns the tree. Workers build standalone chunk storage (generation,
decoding, compaction) and send it over a channel; the main thread grafts it in.
Each chunk has a generation counter, and each frame collects the bricks every
edit batch changed, for decision 0007's uploads.

### Work this requires in World_Core, in order

1. Decoding by construction instead of one voxel at a time.
2. A bottom-up builder from column heights, dense bricks and bounded functions.
3. The not-resident child state.
4. Per-chunk storage, and per-chunk compaction in depth-first order.
5. Overlays (KEEP, diff, merge) and coarse summaries per chunk.
6. A byte breakdown in `stats()`, to decide whether a later format version should
   store fully solid leaves more compactly.

## Consequences

- RAM scales with the 64 m disc, not the world, and disk scales with what players
  destroyed. A long session of heavy destruction can still grow saves to hundreds
  of megabytes.
- The terrain generator becomes part of the save format: a save is only valid
  against the generator version it was made with.
- Two problems measured today become per-chunk: fragmentation is fixed by
  rebuilding one chunk, and the first edit after a snapshot copies one chunk's
  directory instead of the whole tree's.
- Every reader must handle not-resident children, and tests must cover grafting,
  eviction and edits to chunks that are not loaded.
- Streaming adds latency: a fast player can outrun generation, so the coarse tier
  and the GPU's fallback to coarser data (decision 0007) must cover the gap.
