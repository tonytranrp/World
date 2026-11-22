# 0031: Two-material leaves are stored inline in their brick

Status: accepted, 2026-11-21

## Context

After decision 0025, a leaf of exactly two materials in the CPU tree was a
three-word record in its brick's block (leaf mask, then a payload word
holding the offset of its palette block) plus a palette block allocated
apart in the chunk's word arena: the palette word (both ids) and one index
bit per solid voxel. On the 107 full-detail chunks of decisions 0021 and
0025 (`chunk_bytes`, World_Gen):

| | Count |
|---|---|
| Two-material leaves | 2,834,915, in 217,348 of 698,941 bricks |
| with more than 32 solid voxels (two index words) | all of them |
| of which one material is the brick's default | all of them |
| Their bytes: 12 B record and 12 B block each | 64.9 MiB of 154.6 MiB live (42%) |

Decision 0025 listed these 8.5 million block words as the last large item.
Every such leaf paid a payload word that only pointed to the next block,
and a separate allocation per leaf, which also scattered the palette
blocks from their brick.

## Decision

**CPU tree** (World_Core `tree/node`). A level-1 header gains an optional
*pair mask* `P` (two words after the default material and, when present,
the full mask): every leaf of exactly two materials. Its `info` widens the
block length to nine bits (bits 0 to 8, a brick now holds up to 327 words)
and moves `BRICK_FULL_MASK` to bit 9; `BRICK_PAIR_MASK` is bit 10. A pair
leaf is five words where its record would be:

| Word | Content |
|---|---|
| 0, 1 | leaf mask |
| 2 | palette word: the lower id in the low half, the higher in the high half |
| 3, 4 | one index bit per solid voxel in slot order (word 4 zero up to 32 solid voxels) |

so stored leaf `i` starts at `header + rank(mask & !A, i) + 2 * rank(mask &
!B, i) + 2 * rank(P, i)`. Words 2 to 4 are exactly a two-material palette
block as decision 0025 lays it out, so `NodeView::palette_block` still
returns the GPU's block (a slice of the brick's own words, its exact
length) and World_Render's encoder copies it as before: GPU bytes, the
encoder and the shaders do not change. Leaves of three or more materials
keep their record and their block apart. A brick without two-material
leaves is byte for byte what it was, but for the flag's new bit.

In memory a two-material leaf is a `Record` whose new `indices` field is
not zero (both materials are present, so some index is 1); its payload is
the palette word. Point lookups (`step`, `TreeCursor`) gain a `Pair` step
read in one five-word slice; a brick without the full and pair masks keeps
the one-popcount path of decision 0021. Edits that turn a pair into
another pair rewrite its five words in place; everything else repacks the
brick as before.

**Allocator bookkeeping.** The word allocator kept one free list slot per
block length, an array of `MAX_BLOCK_LEN` `Option<Arc>`s (197 then, 327
now: 1.6 then 2.6 KiB per chunk, allocated for every chunk, also the
coarse ones whose median content is now 1 KiB). Its free lists are now a
sorted vector of the lengths that have one (a built chunk has only its
page tails), so a chunk's allocator is about 50 bytes and 32 more per
length it has freed.

## Alternatives considered

- **One material per brick for pairs in the header** (every pair here
  shares the default; the other could be one header word): four words a
  leaf, 19 MiB less instead of 9, but the leaf's palette block would no
  longer be contiguous, so the GPU encoder would scan 64 voxels per leaf
  (about 17 ms more encoding per chunk) unless World_Render read a new
  accessor; not this change.
- **Tagging pair payloads instead of a header mask**: a record's length
  must follow from the header to find leaf `i` by popcounts.
- **A per-chunk dictionary of leaf masks** (the 5.2 million default partial
  leaves have 43,029 distinct masks): could save more than 20 MiB, but needs
  reference counting on every edit and an indirection on every read; not
  now.

## Consequences

`cargo test --release -p world_gen --test chunk_bytes -- --ignored
--nocapture`, before (2d2f604) and after:

| 107 full-detail chunks | Before | After | Change |
|---|---|---|---|
| CPU tree, live | 154.6 MiB | 145.5 MiB | -9.1 MiB, -5.9% |
| CPU tree, pages held | 154.8 MiB | 145.6 MiB | -5.9% |
| Chunk median (p10, p90) | 1,464 KiB (639, 2,333) | 1,374 KiB (638, 2,148) | |
| Palette blocks apart | 2,834,915 | 0 | |
| Bricks with a pair mask | | 217,348 (1.7 MiB of header words) | |
| Build time of all 107, one thread | 4.2 to 4.7 s | 3.5 s | |

The saving is the 2,834,915 payload words (10.8 MiB) less the 217,348
pair masks (1.7 MiB). The coarse ring has no leaves and does not change.
World_Render's `terrain_bytes` gives the same CPU figures, the encoder
agrees with the tree, and the GPU brick blocks stay 145.5 MiB (1,392 KiB a
chunk), byte for byte as before.

Times, measured on prebuilt executables of 2d2f604 and this change,
alternating (`chunk_reads`, `terrain_tree/chunk_build_only` and
`workloads`' bottom-up reads, terrain serialization, `apply_1k`,
`overwrite_dense`), three rounds with 1 s warm-up and 3 s measurement, then
the point reads alone in five more rounds. The machine was loaded by two
other builds and moved 20 to 40% between rounds of the same executable, so
only medians and consistent directions are given:

| Bench | Change, median |
|---|---|
| chunk_reads/walk_every_brick (the encoder's access pattern) | -8% |
| terrain_tree/chunk_build_only | -3% and -10% (two runs) |
| serialize/decode_terrain_512 | -7% and -1% |
| serialize/encode_terrain_512 | -4% and +13% |
| apply_1k, overwrite_dense/set_10k, chunk_edits/crater | -2% to +5%, both directions |
| chunk_edits/scattered_1k_and_restore | +5% and +19% |
| point reads: chunk_reads get and cursor 100k, bottom-up line, brick, scattered 10k | -14% to +9%, both directions between runs |

- Building and decoding allocate one block per brick instead of one more
  per two-material leaf, and a brick's leaves are now walked in order:
  `NodeView::children` of a brick reads the header once and each leaf where
  the previous one ended, instead of ranking every slot (a separate
  internal change, no API change, measured with this one).
- Point reads: a brick with a pair mask and no full mask (the common case
  here) has its own short path in `step`, the old one-popcount path stays
  for bricks with neither mask, and a pair leaf is read in one five-word
  slice instead of a record and a block elsewhere. No consistent change
  showed under this machine's noise; an instruction count on Linux would
  settle it.
- Scattered single-voxel edits may be slower: a leaf that changes between
  one and two materials now changes its brick's block length (five words
  instead of three), so the brick repacks where a record and a separate
  palette block used to change apart.

- Saves are unaffected: serialization walks the read interface, so encoded
  bytes, the format version, content hashes and World_Gen's golden hashes
  do not change. `same_content`, the canonical walks (World_Core's tests
  and World_Testkit's) and `memory_breakdown` (new `pair_leaves` and
  `pair_headers`) count the new layout; `compare_after_edits` against
  `DenseWorld` and the property tests pass unchanged.
- The instruction-count gate (gungraun, Linux) was not run.
- Next: the 5.2 million default partial leaves (39.8 MiB, 27% of what is
  left) repeat 43,029 distinct masks; the 2.0 million other full leaves as
  16-bit halves (3.8 MiB); a default for bricks without full leaves (2.7
  MiB, decision 0025).
