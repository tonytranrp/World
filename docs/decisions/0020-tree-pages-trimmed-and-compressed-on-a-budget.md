# 0020: Built chunks trim their last pages; evicted chunks compress on a budget

Status: accepted, 2026-11-21

## Context

Decision 0014's heap audit of the floor walk (2019 MacBook Pro, i7-9750H,
Radeon Pro 5300M, `--mac-floor`, 8 m/s) found World_Core's tree pages
(`push_page`) the largest heap item: 306 MiB at frame 1,000, holding hot
chunks, the coarse tier and chunks built and waiting to join. The warm tier
(compressed evicted chunks) had stayed empty because the residency workers
were never idle. This record measures what those pages are.

**Page fill of real chunks.** The default terrain (`assets/config/terrain.ron`,
seed `0x5EED`), every chunk the surface crosses and one below, built as the
workers build them (`build_region` then `take_chunk`), measured with
`stats()` and `memory_breakdown()` (release build):

| | Full detail: 97 chunks within 64 m | Coarse, cell level 2: 2,022 chunks from 64 to 384 m |
|---|---|---|
| Live bytes (nodes x16 + words x4) | 177.1 MiB, 1,872 KiB a chunk (median; p10 775, p90 2,967) | 155.0 MiB, 87 KiB a chunk (median; p10 34, p90 107) |
| Words on free lists and page tails | 0.43 MiB (0.24%) | 0.48 MiB (0.3%) |
| Pages held (whole 16 KiB pages) | 179.1 MiB: 1.1% slack | 191.5 MiB: 19% slack |

- The builder writes each block once at its final size, children before
  parents, so a built chunk has no holes and compacting it before it joins
  would gain nothing; `take_chunk` and `insert_chunk` move these arenas as
  they are.
- The only slack is each arena's last page, half of 16 KiB on average per
  arena. Against a 1.8 MiB full-detail chunk that is 1%; against an 87 KiB
  coarse chunk it is a fifth.
- Uniform subtrees are collapsed (the tree is canonical, decision 0004). A
  full-detail chunk's bytes are leaf records more than anything:

  | Full-detail content (97 chunks) | Count | MiB | Share |
  |---|---|---|---|
  | Records of one material with air (surface leaves), 12 B | 5,471,812 | 62.6 | 35% |
  | Palette blocks, all two materials at one bit, 4 words each | 2,560,528 | 39.1 | 22% |
  | Palette records, 12 B | 2,560,528 | 29.3 | 17% |
  | Full records not of their brick's default material, 12 B | 1,825,597 | 20.9 | 12% |
  | Bricks, 16 B node and 12 B header | 627,193 | 16.8 | 9% |
  | Nodes of level 2 and up and their child words | 41,606 | 8.5 | 5% |
  | Full leaves of the brick's default material | 19,552,949 | 0 | |

  A coarse chunk at cell level 2 is nodes of level 2 to 4 only, and 92% of
  its bytes are child words: one word for every solid 16-voxel cell of a
  level-2 node that is not one material, which near the surface (grass,
  soil and rock within a metre) is most of them.

**Where the pages are in the walk.** The residency log (now also printing
the chunks waiting to join and to be compressed) and live allocations by
call stack at frame 1,000, release build, 3,000-frame walk at 8 m/s, load
averages 11 to 15, CPU speed limit 100:

| Holder | Logged | Heap (pages, with malloc's rounding) |
|---|---|---|
| Hot chunks (437 loaded, air and one-material chunks among them) | 428 MiB | 502 MiB of pages built by `generate_cached`, together |
| Evicted, unchanged, waiting to be compressed | 67 chunks, 47 MiB | |
| Built and waiting to join | 0 at every five-second sample | |
| Coarse columns | 667 columns, 75 MiB | 91.9 MiB |

- No chunk waited to join: the workers' results are drained every frame
  and at most 16 loads are in flight against 8 grafts a frame.
- The compression queue sat at its bound of 64 chunks (`in_flight_cap * 4`)
  from the second sample on: compressions were handed out only while fewer
  than 16 loads were outstanding, which is never while walking. On this run
  the machine was quiet enough that the queue still drained between bursts
  (the warm tier reached 296 MiB by frame 3,000); on 0014's runs it never
  did, so up to 64 evicted chunks, about 50 MiB, sat in RAM and were then
  dropped uncompressed.
- macOS rounds each 16,400-byte page allocation (16 KiB and the `Arc`
  header) up to 16,896 bytes, 3% more than the logged figures.

## Decision

- **A built chunk trims its arenas' last pages** (`Store::trim`, called by
  `Builder::end_chunk`): the last node page and the last word page keep only
  the items in use, rounded up to 64. Pages are `Arc<[T]>` instead of
  `Arc<[T; N]>`; only the last page may be short, and the first write past
  its end (an edit that allocates there, or a new page) grows it back to the
  full page. Reads index the page slice as before. Every chunk the builder
  makes is trimmed: generated, decoded, coarse, compacted, extracted and
  overlays. `stats().bytes` counts what the pages hold.
- **A compression is always in flight**: one, whatever the loads; more
  while fewer loads than the in-flight cap are outstanding, as before.
  Compressing a chunk (encoding and LZ4) takes a few milliseconds against
  tens for generating one, and it frees about 1 MiB of pages. The measured
  runs below reserved one per worker (four); review cut it to one so
  compressions never hold every worker while loads wait, since one slot
  drains far faster than walking evicts. The figures were not re-measured
  with one.
- The residency log also prints the bytes held for writes, and the chunks
  waiting to join and to be compressed with their bytes
  (`ResidencyStats::ready`, `ready_bytes`).

## Alternatives considered

- **Smaller pages** (4 KiB). Cuts the slack of every chunk by four, but
  quadruples the directory and per-page overhead of the hot chunks, which
  are 85% of the pages, and changes decision 0004's copy-on-write trade.
  Trimming removes the slack where it is, once, and leaves full pages alone.
- **Compacting a chunk before it joins.** Measured: a built chunk has 0.24%
  free words, so there is nothing to compact.
- **A smaller compression queue bound** without a reserve: fewer waiting
  chunks, but every chunk past the bound is dropped instead of kept warm,
  so a walk back regenerates it.
- **Page lengths that fit malloc's 512-byte quantum** (1,023 nodes, 4,092
  words a page): saves the 3% rounding, but makes every page index a
  division by a constant instead of a shift on the read path.
- **Changing the leaf record encoding** (the largest item, see Context).
  The layout is a GPU format (decision 0004); see Consequences.

## Consequences

- The same chunks after trimming: full detail 177.6 MiB held for 177.1 live
  (was 179.1), coarse 156.7 MiB for 155.0 live (was 191.5): a coarse chunk
  holds 18% less.
- Live allocations at frame 1,000, one run each, back to back, release
  build. The second run got further on a quieter machine (1.21 against
  0.93 million brick cache records, 954 against 667 coarse columns), so
  totals are not comparable; per holder:

  | | Before | After |
  |---|---|---|
  | Pages of full-detail chunks | 502.0 MiB (31,152 pages) | 467.9 MiB (29,041) |
  | Waiting to be compressed (logged, nearest sample) | 67 chunks, 47 MiB | 8 chunks, under 1 MiB |
  | Coarse pages | 91.9 MiB, 667 columns: 141 KiB a column | 108.7 MiB, 954 columns: 117 KiB a column |
  | Heap total | 1,595.7 MiB | 1,619.1 MiB |

  Over the whole walk (a sample every five seconds) the chunks waiting to
  be compressed held 40.1 MiB on average before (most 67) and 29.7 MiB after
  (most 70), and the coarse tier ended at 110 against 90 KiB a column.
- Under load the reserve fills the warm tier but does not shrink the queue:
  a pair of walks with load averages 13 to 79 and the CPU speed limit 37 to
  46 kept no warm chunk at all before (the workers were never idle, as in
  0014) and 266 after, while the waiting chunks held 55.4 and 54.8 MiB on
  average; the queue sits at its bound of 64 either way, because evictions
  come in bursts of a chunk slice and the compressions queue behind up to
  16 loads. Peak RSS of that pair, 1,113 and 1,007 MiB, is not comparable:
  the second run loaded 232 chunks against 373. The bound is the lever for
  those bytes; lowering it needs the warm cache test that evicts a whole
  sphere in one step to change (with no workers the bound is 16).
- `cargo bench -p world_core --bench tree` and `workloads` (1 s warm-up,
  3 s measurement) could not resolve a difference: the machine's load
  average moved between 25 and 90 during the runs, and benchmarks of code
  this change does not touch (`chunks/compress_Lz4`, `decompress_Zstd`)
  moved 25 to 75% between the before and after runs. No benchmark was
  slower than its before figure except `chunks/take_and_insert_dense_chunk`
  (+42% while untouched code ran 25 to 30% faster), which builds a tree's
  first pages: they were filled through an iterator, and are built as an
  array again, as before; rerun, it measured 27% faster than before while
  the LZ4 control ran 53% faster, which this machine cannot separate from
  load. The instruction-count gate (Linux, `instructions.rs`) was not run
  here. `cargo bench -p world_core --bench memory` prints the same figures
  before and after: its scenes are built by edits, which do not trim.
- Reads now index a slice instead of an array, one bounds check per
  uncached page read (`PageReader` already held slices).
- A trimmed chunk's first allocation in its last page copies that page
  back to full size, at most one 16 KiB copy per arena per chunk; the
  copy-on-write after a snapshot copies the same page anyway.
- Next, by size: the leaf records (a full record not of the brick's default
  material stores an all-ones mask, 8 of its 12 bytes; surface records and
  two-material palettes are 74% of a full-detail chunk) are a GPU format
  change; coarse cells that no ray can reach (solid on every side) could
  take their column's surface material so buried level-2 nodes collapse;
  the band between `full_radius_m` 64 and `evict_radius_m` 96 is, by
  surface area, more than half of what the hot tier may hold (not
  measured); a smaller compression queue bound; and page
  allocations rounded by malloc (3%).
