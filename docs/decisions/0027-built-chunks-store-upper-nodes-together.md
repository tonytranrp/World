# 0027: Built chunks store their upper nodes together, breadth first

Status: accepted, 2026-11-21

## Context

Every chunk the game holds is made by World_Core's bottom-up builder
(`tree/mutate/build.rs`): generated and coarse chunks, decoded warm and
saved chunks, compactions, copies, extracts and overlays. The builder
writes each node once, children before parents, allocating from the end of
a fresh chunk's arenas. So a chunk's arenas were in post-order:

| Arena | Order |
|---|---|
| Nodes | up to 64 bricks, their level-2 node, the next bricks, the next level-2 node, ..., the level-3 nodes among them, the root last |
| Words | each brick's palette blocks then its block, ..., each level-2 block after its bricks' blocks, ..., the root's block last |

A point lookup reads the root, a level-3 and a level-2 node and their child
words, then the brick node and its block (and a palette block). In
post-order each upper node sat after its subtree, so the about 440 upper
nodes of a surface chunk (46,842 for the 107 chunks of decision 0025) and
their 2.3 million child words were spread over the whole 1.4 MiB chunk:
every level of a lookup was a different node page and word page (each a
miss of `PageReader`'s one-page cache, two dependent loads through the page
directory) and a different cache line. The baseline
(`docs/research/perf-baseline-windows-core.md`) also showed the builder's
layout reading no faster than a tree made by edits
(`coherent_reads/bottom_up_*` against `terrain_*`).

## Decision

In a chunk the builder opened itself (`begin_chunk`, which every path above
uses), `Builder::branch` holds the nodes of levels 2 to 4 instead of
storing them: their masks and child words go into a buffer, and their
parents name them with a marker bit (bit 30, far above any real node index
of a chunk). Bricks and their palette blocks are stored as before, as they
come, so each brick's palette blocks still lie just before its block and
the bricks of one level-2 node are contiguous. `end_chunk` then stores the
held nodes breadth first from the chunk root: first every node (root,
level-3 nodes, level-2 nodes), so their indices are known, then every block
in the same order with the marked words replaced by those indices.

| Arena | Order now |
|---|---|
| Nodes | every brick, depth first; then the root, the level-3 nodes, the level-2 nodes |
| Words | each brick's palette blocks then its block, depth first; then the root's block, the level-3 blocks, the level-2 blocks |

The upper nodes of a chunk fit one node page (440 nodes of 1,024) and
their blocks a few word pages, so a lookup's three upper levels mostly
share pages and stay in cache across lookups. Nothing about the format
changes: the same nodes and words, at other indices. A builder working
inside an existing chunk (`build_region` below the chunk level, grafts)
stores nodes as before.

## Alternatives considered

- **Depth-first pre-order** (each node before its children). The same
  pages for the upper levels, but bricks would interleave with level-2
  nodes again, and the builder cannot know a parent's index before its
  children exist without the same holding step.
- **A relayout pass at `end_chunk`** copying the whole chunk into new
  arenas: puts bricks in any order too, but costs a full copy of 1.4 MiB per
  chunk; holding only the upper nodes copies about 2% of the words.
- **Bricks next to their block** (one arena): a format change, the GPU
  encoder and every reader would follow.

## Consequences

Measured back to back on prebuilt bench executables of 27fb3b7 and this
change, alternating, three rounds each (`chunk_reads`, `terrain_tree`,
`coherent_reads/bottom_up`), Ryzen 9 5980HX shared with two other builds.
The first round's "after" ran during a quiet spell (everything, untouched
code included, 15 to 50% faster), so the table gives rounds 2 and 3:

| Bench | Before | After | Change |
|---|---|---|---|
| chunk_reads/get_100k_near_surface (25 generated chunks) | 12.6, 12.2 ms | 11.7, 11.4 ms | -7% |
| chunk_reads/cursor_100k_near_surface | 14.7, 13.6 ms | 13.4, 13.6 ms | -4% |
| chunk_reads/cursor_rays_256 | 214, 220 µs | 209, 205 µs | -4% |
| chunk_reads/walk_every_brick | 220, 221 ms | 220, 218 ms | -1% |
| chunk_edits/crater_r12_and_restore, scattered_1k | 553, 563 µs; 1.33, 1.33 ms | 572, 563 µs; 1.32, 1.32 ms | 0 |
| coherent_reads/bottom_up_line_512 (bench terrain) | 18.8, 19.2 µs | 15.6, 15.8 µs | -17% |
| coherent_reads/bottom_up_surface_brick_16 | 192, 195 µs | 178, 180 µs | -7% |
| coherent_reads/bottom_up_scattered_10k | 808, 810 µs | 725, 722 µs | -10% |
| terrain_tree/chunk_build_only (one surface chunk) | 21.8, 21.8 ms | 22.6, 22.4 ms | +3% |

- Point reads and ray walks on builder-made chunks are 4 to 17% faster;
  the brick walk (the GPU encoder's access pattern, which visits bricks in
  order and was already sequential) and edits do not change. Building a
  chunk costs about 3% more (a second pass over the upper nodes and a
  buffer of their words).
- Bytes do not change: `chunk_bytes` gives the same 154.6 MiB live for the
  107 chunks and 154.8 MiB of pages; the page tails left by blocks that do
  not fit a page's end hold 20,233 free words instead of 22,446.
- Edits allocate new blocks at the end of the arena or from free lists as
  before, so an edited chunk drifts from this layout until it is rebuilt
  (decoded from the warm tier or compacted).
- The instruction-count gate (gungraun, Linux) was not run.
- Next: the brick node (16 bytes in the node arena) and its block are still
  in different pages; reading them together needs a format change.
