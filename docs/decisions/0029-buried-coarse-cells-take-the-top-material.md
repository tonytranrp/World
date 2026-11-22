# 0029: Buried coarse cells take their column's top material

Status: accepted, 2026-11-21

## Context

The coarse tier (decision 0006, `coarse_radius_m` 384 m in
`residency.ron`) builds distant chunks from World_Gen's `CoarseSource`:
every aligned cell of `4^k` voxels (`coarse_cell_level` 2: 16 voxels, a
quarter metre) is one material, its column sample's material at the cell's
middle height. A coarse chunk is then nodes of levels 2 to 4 and their
child words only. Decision 0020 found child words 92% of a coarse chunk's
bytes: one word for every solid cell of a level-2 node (64 cells) that is
not all one material, which near the surface is most of them, since grass,
soil and rock alternate within a few metres. It proposed that cells no ray
can reach take the column's surface material so those nodes collapse.

Measured with the new `chunk_bytes` test (World_Gen, `--ignored`): the
coarse ring at cell level 2, every footprint whose centre lies 64 to 384 m
from the origin, the chunks the surface crosses and one below, 3,604
chunks: 155.8 MiB live, of which 37.6 million child words (143.6 MiB, 92%)
and 801,258 nodes. That is as much as the 107 full-detail chunks within
64 m (154.6 MiB).

## Decision

`CoarseSource` marks a cell *buried* when the cell above it in its column
and the cells beside it in the four neighbouring columns are solid (side
neighbours outside the footprint count as the nearest sample inside it). A
buried cell takes its column's top material: the material of the column's
highest solid cell. Visible cells keep their sample's material, so every
face a ray can reach shows what it showed before.

Below the surface cover, a level-2 node whose 16 columns share a top
material is then one material and collapses into a single uniform child
word of its level-3 parent; deeper level-3 and level-4 nodes collapse
likewise. `CoarseSource::uniform` also answers "all one material" for a
cube whose every cell is buried in columns of one top material, so the
builder stops there instead of descending to the cells and packing them.
The full-detail chunks, World_Gen's generated voxels and its golden hashes
(which cover the coarse column samples, not this source) do not change.

## Alternatives considered

- **Buried cells take bedrock** (the deepest layer): rock below the soil is
  bedrock already, so fewer cells change, but measured on the same ring it
  keeps 85.2 MiB live against 56.5 MiB: the nodes right below the surface
  cover mix the top material of the visible cells above them with bedrock.
- **Exposure across the footprint edge from the neighbouring footprint's
  samples.** Exact at chunk-border cliffs, but needs a border of samples
  per footprint (256 more column queries a coarse chunk) and an API change
  for the residency workers. With the nearest sample standing in, a cliff
  exactly on a footprint edge can show its top material on the side faces
  of the cells next to the edge, at 64 m and farther, at a quarter metre a
  cell; not worth the queries now.
- **A coarser cell level** (`coarse_cell_level` 3): 64 times fewer cells,
  but 1 m cells from 64 m on; a visual decision, not this one.

## Consequences

`cargo test --release -p world_gen --test chunk_bytes -- --ignored
--nocapture`, before and after back to back:

| Coarse ring, 3,604 chunks | Before | After | Change |
|---|---|---|---|
| Live bytes | 155.8 MiB | 56.5 MiB | -63.7% |
| Pages held | 157.2 MiB | 57.3 MiB | -63.5% |
| Per chunk, median (p90) | 36 KiB (101) | 1 KiB (39) | |
| Nodes | 801,258 | 418,218 | -48% |
| Child words | 37,640,005 | 13,125,445 | -65% |
| Build time, all chunks, one thread (columns included) | 29.8 s | 21.7 s | -27% |

- In the floor walk of decision 0020 the coarse tier held 90 to 117 KiB a
  column (75 to 109 MiB); the same columns should hold about a third of it,
  so `coarse_cap_mib` (500) has room for a farther `coarse_radius_m` or can
  come down. Neither is changed here; the walk was not rerun.
- The GPU mirrors fewer coarse nodes (World_Render's node table holds
  levels 2 to 4 of coarse chunks), and each coarse chunk uploads fewer.
- Chunk summaries (decision 0022) of coarse nodes count buried cells as
  the top material, so a distant node's representative material is now
  mostly what covers it (grass rather than the rock under it). Rays that
  stop at a summary see that colour; full-detail and visible coarse cells
  are unchanged.
- `tree_build`'s coarse test now expects buried cells to hold the top
  material, and a new test checks every cell that differs from its sample
  is solid above and on all four sides.
- Next: the remaining coarse bytes are the surface level-2 nodes, a word
  per solid cell; a level-2 node whose cells are all air above one height
  and one material below could be stored as a column of heights, but that
  is a format change on both sides.
