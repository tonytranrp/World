# 0021: Full leaves store no mask, in the CPU tree and in GPU brick blocks

Status: accepted, 2026-11-21

## Context

Decision 0020 measured real terrain chunks and found leaf records 74% of a
full-detail chunk's bytes. One item was plainly redundant: a full
one-material leaf that is not its brick's default material spent 8 of its
12 bytes on an all-ones mask. This record writes down both leaf encodings
as they were, measures them on the same chunks, and changes how full
leaves are stored.

**CPU tree before** (World_Core `tree/node`, decisions 0004 and 0005). A
level-1 node (16 bytes: mask, base, `info` = number of records) owns one
block of words:

| Words | Content |
|---|---|
| 0, 1 | default mask `D`: the leaves full of the default material; no words |
| 2 | default material: the most common material of the full one-material leaves, ties to the lower id |
| 3 + 3k | one record per other non-air leaf, in slot order: leaf mask lo, hi, payload |

A payload with bit 31 set is the one material of the leaf's solid voxels;
otherwise it is the word offset of a palette block (header `p | b << 8`,
the palette two ids a word, then a `b`-bit index per solid voxel, LSB
first). Leaf `i`'s record is at `3 + 3 * rank(mask & !D, i)`.

**GPU brick block before** (World_Render `brick_cache/format`, decision
0007). It is not the CPU's bytes: the encoder transcodes through the read
interface and copies only palette blocks verbatim.

| Words | Content |
|---|---|
| 0 | brick id |
| 1 | `n \| palette_words << 8 \| flags << 24` |
| 2 | representative material |
| 3 | reserved |
| 4 .. 4+2n | the leaf mask of every non-air leaf |
| 4+2n .. 4+3n | the payload of every non-air leaf |
| then | palette blocks, padding to 16 bytes |

Bytes per leaf kind:

| Leaf | CPU | GPU |
|---|---|---|
| full, the brick's default material | 0 | 12 |
| full, another material | 12 (mask all ones) | 12 |
| one material with air (surface) | 12 | 12 |
| two or more materials | 12 + palette block (16 for two at one bit) | 12 + palette block |

So the GPU, unlike the CPU tree, paid 12 bytes for every full leaf, and
most leaves of a surface brick are full.

**Measured** on the default terrain (`terrain.ron`, seed `0x5EED`): every
chunk the surface crosses, and one below, whose footprint centre lies
within 64 m of the origin, built as the workers build them
(`build_region`, `take_chunk`). That is 107 chunks (0020 counted 97 with a
slightly different footprint rule; the per-chunk median, p10 and p90 agree
to 1 KiB). `crates/Engine/World_Render/tests/terrain_bytes.rs` reproduces
it: `cargo test --release -p world_render --test terrain_bytes --
--ignored --nocapture`.

| 107 chunks | Count |
|---|---|
| Bricks | 698,941 |
| Full leaves of the brick's default | 21,785,042 |
| Full leaves of another material | 2,011,900, in 131,889 bricks (19%) |
| Surface leaves | 6,114,283, of which 5,219,942 (85%) are of the brick's default |
| Two-material leaves | 2,834,915, all at one bit |

## Decision

**CPU tree.** A level-1 node's `info` holds its block's length in words
(bits 0 to 7) and a flag, `BRICK_OTHER_FULL` (bit 8). The header is the
default mask and material as before, and, only when the flag is set, a
second mask `F`: every other full one-material leaf. Stored leaves follow
in slot order: a leaf in `F` is one word (bit 31 and its material, the same
word as a uniform child), every other non-default leaf a three-word record
as before. Leaf `i` starts at `header + rank(S, i) + 2 * rank(S & !F, i)`
with `S = mask & !D`. A brick without other full leaves (81% of them) is
byte for byte what it was, so its header stays three words.

**GPU brick block.** The header grows from 4 to 8 words and carries the
same split:

| Words | Content |
|---|---|
| 0 | brick id |
| 1 | `p \| palette_words << 8 \| flags << 24`, `p` = partial leaves |
| 2 | representative material |
| 3 | default material (same rule as the CPU's) |
| 4, 5 | default mask `D`: no words |
| 6, 7 | full mask `F`: a payload, no leaf mask |
| 8 .. 8+2p | leaf masks of the partial leaves `P = mask & !D & !F` |
| 8+2p .. 8+2p+s | payloads of the stored leaves `S = mask & !D` |
| then | palette blocks, padding to 16 bytes |

The traversal (`traversal/tree.wesl`) reads `D` first and returns the
default material for a hit in a default leaf without touching the leaf
words; otherwise one payload, and the leaf mask only for a partial leaf.
The hit (voxel, face, distance, level 0, material) is the one the old path
gave, since a full leaf's mask was all ones. The four new constants come
from World_Render through `cargo xtask codegen` (decision 0013).

**Saves are unaffected.** Serialization walks the read interface (decision
0004), so the encoded bytes, the format version, content hashes and
World_Gen's golden hashes do not change; decoding builds the new layout.
Generated voxels are unchanged (the census above is identical before and
after). The warm tier and region files store serialized chunks.

## Alternatives considered

- **A fixed five-word CPU header.** Simpler reads, but 8 more bytes for
  every brick: 5.3 MiB on these chunks against 1.0 MiB with the flag.
- **One full mask without a default on the GPU** (every full leaf a
  payload): a 4-word header saving, but 4 bytes for each of 21.8 million
  default leaves, 83 MiB.
- **Full-leaf materials as 16-bit halves.** Saves another 2 bytes per other
  full leaf (4 MiB) at the cost of a second array and half-word addressing
  on both sides.
- **Sharing one layout between the CPU tree and the GPU.** It would remove
  the transcoding, but the CPU block is edited in place and its records are
  addressed with the flag in `info`, while the GPU block wants every count
  in its own header; not this change.

## Consequences

Bytes of the 107 chunks, before and after (release build; the counts above
do not change):

| | Before | After | Change |
|---|---|---|---|
| CPU tree, live | 196.8 MiB | 182.5 MiB | -14.3 MiB, -7.3% |
| CPU tree, pages held | 197.3 MiB | 182.7 MiB | -7.4% |
| CPU chunk median (p10, p90) | 1,873 KiB (775, 2,967) | 1,720 KiB (774, 2,692) | |
| GPU brick blocks, every brick resident | 430.1 MiB (4,115 KiB a chunk) | 177.1 MiB (1,695 KiB a chunk) | -253 MiB, -58.8% |

- The GPU brick pool holds the bricks rays ask for, not all of them, but
  each one is on average 2.4 times smaller, so the same pool holds 2.4
  times as many bricks and each upload moves 59% fewer bytes. The pool
  size in `brick_cache.ron` can come down on the floor Mac; not changed
  here.
- The CPU saving is the 2.0 million other full leaves at 8 bytes, less the
  131,889 longer headers at 8 bytes.
- Reads on the CPU: a brick step reads `info`'s flag and, for a brick with
  other full leaves, two more header words and one more popcount. A hit in
  an other full leaf now ends at the brick step (one word) instead of
  reading a three-word record.
- `cargo bench -p world_core --bench tree` and `workloads` (coherent
  reads, `apply_1k`, serialize, carve; 1 s warm-up, 3 s measurement),
  before (fcbd145) and after, back to back, load averages 25 to 42, CPU
  speed limit 100, cannot resolve the change: `tree_snapshot`, which this
  change does not touch, measured 46% faster in the second run, and the
  other figures moved from 54% faster (`coherent_reads/terrain_line_512`)
  to 26% and 38% slower (`tree_fill_64_boxes`, `serialize/decode_dense_128`).
  These synthetic scenes have few bricks with two full materials, so their
  read path is the old one plus one flag test. The instruction-count gate
  (Linux, `instructions.rs`) was not run; it would see that flag test in
  every brick step, two more header loads and a popcount in bricks with
  other full leaves, and the GPU encoder's extra pass over a brick's 64
  children (World_Render's `encode` bench was not run, for disk space).
- GPU reads: the brick header is now two 16-byte rows instead of one;
  default leaves skip the leaf mask and payload loads.
- `MemoryBreakdown` gains `long_headers` and counts other full leaves at 4
  bytes; World_Testkit's canonical walk counts the new word layout.
  `cpu_cache`'s budget test now uses striped bricks: its slab's bricks
  became a few words, smaller than the fixed per-brick writes the budget
  also pays for. `gpu_trace`'s world-tree scene gains a brick with full
  leaves of two materials, so the CPU-vs-GPU hit comparison covers the
  default mask, the full mask and partial leaves in one brick.
- Next, by size (same chunks, after this change): surface leaves of the
  brick's default material could drop their payload word (5.2 million
  leaves, 4 bytes each, about 20 MiB on each side less a third mask's
  header words); two-material palette blocks always start with the same
  header word (`2 | 1 << 8`), 2.8 million words, 11 MiB on each side;
  representative material and default material could share header word 2
  on the GPU.
