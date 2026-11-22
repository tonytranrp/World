# 0037: A dictionary of leaf masks: measured, not now

Status: rejected for now, 2026-11-21

## Context

After decisions 0031 to 0035 the largest item in a full-detail chunk is
leaf masks: on the 107 chunks of `chunk_bytes` (141.7 MiB live), 5,219,942
partial leaves of their brick's default material are a mask alone (8 bytes,
39.8 MiB, 28%), and every other partial leaf (894,341 single-material
records, 2,834,915 two-material leaves) starts with one too: 8,949,198
masks, 68.3 MiB, 48% of the tree. Decision 0031 counted 43,029 distinct
default partial masks over all chunks and suggested storing indices into a
dictionary instead.

## Measured

`cargo test --release -p world_gen --test chunk_bytes leaf_mask_census --
--ignored --nocapture` (new): distinct masks per chunk and over all chunks,
and how many leaves the most common masks of all chunks cover.

| | Masks | Distinct, all chunks | Distinct per chunk: median, p90, max (sum) | Top 256 / 4,096 / 65,536 cover |
|---|---|---|---|---|
| Default partial leaves | 5,219,942 | 43,029 | 703, 7,835, 14,139 (331,134) | 52.5% / 85.3% / 100% |
| Every partial leaf | 8,949,198 | 43,393 | 708, 7,989, 14,468 (338,283) | 69.2% / 90.8% / 100% |

Terrain surfaces repeat a few tens of thousands of shapes, and every chunk
fits a 16-bit index. What a per-chunk dictionary would save on these
chunks (a dictionary entry 8 bytes):

| Variant | Indices | Dictionaries | Saving | Share of 141.7 MiB |
|---|---|---|---|---|
| Default partial masks, 32-bit index | 19.9 MiB | 2.5 MiB | 17.4 MiB | 12% |
| Default partial masks, 16-bit index | 10.0 MiB | 2.5 MiB | 27.3 MiB | 19% |
| Every partial mask, 32-bit index | 34.1 MiB | 2.6 MiB | 31.6 MiB | 22% |
| Every partial mask, 16-bit index | 17.1 MiB | 2.6 MiB | 48.6 MiB | 34% |

## Decision

Not in this round. The saving is real and the largest left, but each
variant needs work this round could not measure safely:

- **Reads.** Every partial leaf read (`step`, `TreeCursor`, the brick walk
  the GPU encoder and summaries use) gains a dependent load from another
  array. CI's instruction gate just failed `get_10k` at +6.3% for one extra
  rank and branch (decision 0035); an indirection on the same path needs
  the gate run on a prototype before it is worth building.
- **Edits.** A changed mask must be interned. A hash map per chunk is
  copied whole by the first insertion after each frame's snapshot (125 KiB
  and more for a p90 chunk, every frame a chunk is dug into); appending
  without deduplicating (only the builder and compaction deduplicate) keeps
  edits a copy of one page, but grows the dictionary by 8 bytes for every
  new mask an edit makes until the chunk is rebuilt.
- **16-bit indices** need a fallback for a chunk whose edits pass 65,536
  masks, half-word offsets in every brick and a GPU-side reader that does
  not care (the encoder reads masks through `NodeView`, so it does not).

## Next

A prototype for default partial masks only, 32-bit indices, append-only
per-chunk dictionaries in copy-on-write pages, deduplicated when a chunk is
built, decoded or compacted: measure `get_10k`, `snapshot_then_edit` and
the destruction benches on the gate, and the dictionary growth over a long
churn (`memory` bench). If reads hold, widen it to every partial mask and
then to 16-bit indices.
