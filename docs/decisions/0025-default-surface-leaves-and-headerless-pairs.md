# 0025: Surface leaves of the default store no payload; two-material palettes no header

Status: accepted, 2026-11-21

## Context

Decision 0021 left full leaves without a mask and listed what was next by
size, on the same 107 terrain chunks (`terrain.ron`, seed `0x5EED`,
`crates/Engine/World_Render/tests/terrain_bytes.rs`):

| 107 chunks, after 0021 | Count |
|---|---|
| Bricks | 698,941, 131,889 with a five-word header |
| Surface leaves (one material and air) | 6,114,283 |
| of them of the brick's default material | 5,219,942 (85%) |
| in bricks without any full leaf | 894,341 (the other 15%, all of them) |
| Two-material leaves | 2,834,915, all at one bit |
| CPU tree, live | 182.5 MiB |
| GPU brick blocks, every brick resident | 177.1 MiB |

A surface leaf of the default material stored its payload word,
`UNIFORM_BIT | default`, which the header already says: 5.2 million words,
about 20 MiB on each side. Every two-material palette block started with
the same header word, `2 | 1 << 8`: 2.8 million words, 11 MiB on each
side. And the GPU block header kept the representative material and the
default material in two words, each 16 bits.

## Decision

**CPU tree** (World_Core `tree/node`). The level-1 header keeps its words
and its flag bit; the masks change meaning:

| Word | Content |
|---|---|
| 0, 1 | default mask `A`: every leaf whose solid voxels are all the default material, full or not |
| 2 | default material (0021's rule: the most common full one-material leaf, ties to the lower id) |
| 3, 4 | full mask `B`: every full one-material leaf; only with `BRICK_FULL_MASK` (bit 8, 0021's `BRICK_OTHER_FULL`) |

The header stores `B` only when it differs from `A`, that is when the
brick has a full leaf of another material or a partial leaf of the
default. A leaf's words follow from its two bits:

| In `A` | In `B` | Leaf | Words |
|---|---|---|---|
| yes | yes | full of the default | none |
| no | yes | full of another material | 1: `UNIFORM_BIT` and the material |
| yes | no | partial, of the default | 2: the leaf mask |
| no | no | anything else | 3: leaf mask and payload |

so stored leaf `i` starts at `header + rank(mask & !A, i) + 2 *
rank(mask & !B, i)`, the same two popcounts as 0021's formula, which this
one contains: without default partial leaves `A` is 0021's default mask
and `B` its default and other-full masks together. A brick without the
flag (185,384 terrain bricks, 27%: those whose full leaves are one
material with no partial leaf of it, and those without full leaves) is
byte for byte what it was.

**Palette blocks, both sides.** A block of two materials has no header: it
starts with its palette word, then its one-bit indices. Readers tell it
from a headed block by the first word's high half, which for a pair is the
larger id (at least 2, since ids are ascending and never air) and for a
header (`p | b << 8`, at most `64 | 8 << 8`) is zero. Blocks of three or
more materials keep their header. The GPU still copies palette blocks
verbatim from the tree (`NodeView::palette_block` says so).

**GPU brick block** (World_Render `brick_cache/format`). The header shrinks
from 8 words to 7 and carries the same masks as the tree:

| Words | Content |
|---|---|
| 0 | brick id |
| 1 | `p \| palette_words << 8 \| flags << 24`, `p` = leaves outside `B` |
| 2 | representative material (bits 0 to 15), default material (16 to 31) |
| 3, 4 | default mask `A` |
| 5, 6 | full mask `B` |
| 7 .. 7+2p | leaf masks of `P = mask & !B` |
| 7+2p .. 7+2p+s | payloads of `S = mask & !A` |
| then | palette blocks, padding to 16 bytes |

The GPU always stores both masks, so a GPU block's `A` holds every partial
leaf of the default (the tree's does too, since its rule has no
threshold). `traversal/tree.wesl` returns the default for a leaf in both
masks, a payload for a leaf in `B` alone, and otherwise reads the leaf
mask and takes the default for a leaf in `A` or the payload's material or
palette entry; `leaf_material` picks the pair layout with `select`. Three
constants are new or moved (`BLOCK_DEFAULT_SHIFT`, `PALETTE_PAIR_SHIFT`,
the mask words), written by `cargo xtask codegen` (decision 0013).

**Point reads.** `TreeView::lookup` walks the branch levels of a chunk in a
loop without the brick step and takes the brick step once after it. With
the flagged-brick code inside the shared loop, LLVM kept less of the
storage reader in registers on every level: probes that end above level 1
(air above the terrain) cost 306 instructions instead of 229, and the
whole `get_10k terrain` scene 36% more. `TreeCursor` keeps the shared step.

**Saves are unaffected.** Serialization walks the read interface (decision
0004), so encoded bytes, the format version, content hashes and World_Gen's
golden hashes do not change, and the voxel census above is identical
before and after.

## Alternatives considered

- **A threshold**: the full mask only from three default partial leaves
  in bricks without other full leaves (two header words against one word a
  leaf). Bricks with one or two such leaves and no other full leaf are
  12,266 and 7,707, so it saves 48 KiB, for a count in every repack and
  in the in-place patch test: a record that is a surface leaf of the default
  can exist in some bricks and not in others.
- **A default for bricks without full leaves**: the most common surface
  material. It turns the other 894,341 surface leaves into leaf masks too:
  CPU 151.9 MiB, GPU 142.2 MiB (2.7 and 3.3 MiB less than this decision),
  but in such bricks a change to a surface leaf can change the default, so
  more edits repack: `set_1k` on an empty tree (1,000 lone voxels) measured
  +5.0% against main, at the CI gate, against +2.1% without it. Left for
  later, with a packer that can afford it.
- **Two bits per leaf in one 128-bit field** instead of two masks: the same
  bytes for flagged bricks, but every read would need a pattern popcount.
- **A payload bit for headerless palettes** (bit 30 of the payload): the
  same saving, but a second encoding of "two materials" on the GPU side
  where the tree's blocks are copied verbatim; the first word already
  tells.
- **Dropping the GPU header's representative word** instead of sharing it:
  nothing reads it, but tests and pool checks print it, and sharing costs a
  shift.

## Consequences

Bytes of the 107 chunks, before (7eb667d) and after, release build:

| | Before | After | Change |
|---|---|---|---|
| CPU tree, live | 182.5 MiB | 154.6 MiB | -27.9 MiB, -15.3% |
| CPU tree, pages held | 182.7 MiB | 154.8 MiB | -15.3% |
| CPU chunk median (p10, p90) | 1,720 KiB (774, 2,692) | 1,464 KiB (639, 2,333) | |
| GPU brick blocks, every brick resident | 177.1 MiB (1,695 KiB a chunk) | 145.5 MiB (1,392 KiB a chunk) | -31.6 MiB, -17.8% |

- The CPU saving is 5,219,942 payload words (19.9 MiB) and 2,834,915
  palette headers (10.8 MiB), less 381,668 more five-word headers (2.9
  MiB): 513,557 of 698,941 bricks now have the full mask.
- `MemoryBreakdown` gains `default_partial_leaves` (8 bytes each) and
  counts palette blocks by their stored length; `long_headers` counts
  bricks with the full mask. World_Testkit's canonical walk counts the new
  words. `terrain_bytes` prints two more census lines.
- Point reads, measured as 0021's note does (`/usr/bin/time -l` instructions
  retired, `Terrain::new(64, 7)` and the bench's 10,000 probes, 200
  repetitions less none, i7-9750H): `get_10k terrain` 3,052,000 on main,
  2,890,000 after (-5.3%). On the same scene after `compact_chunk`, which
  removes page placement from the comparison, 2,948,000 and 2,908,000. The
  reader's accessors stay inline (`nm`). The bench terrain has 9 flagged
  bricks of 33 and 29 default partial leaves.
- The other benches of `instructions.rs`, counted in process around the
  operation (`proc_pid_rusage`, median of 9, same machine), main and after:
  `set_1k` empty 4,667,000 and 4,773,000 (+2.3%), dense 6,652,000 and
  6,715,000 (+0.9%); `fill_box_64` 6,580,000 and 6,674,000 (+1.4%);
  `apply_1k` 3,084,000 and 3,085,000; `snapshot_then_edit` 54,900 and
  50,600; `encode_tree` terrain 3,458,000 and 3,233,000 (-6.5%), dense
  4,189,000 and 4,321,000 (+3.2%); `decode_tree` terrain 3,281,000 and
  3,276,000, dense 3,568,000 and 3,592,000 (+0.7%). Building the bench
  terrain (12,288 column fills, not a gated bench) costs 5.7% more. The
  first packer cost `set_1k` 10% more, mostly a scan that kept too much
  live; the packer now scans for three masks only and visits set bits
  after. The CI gate (Linux, gungraun) was not run; `encode_tree dense`
  is the closest to its 5%.
- GPU reads: both header masks are read for every brick hit (0021 read the
  full mask only after missing the default mask); a default partial leaf
  needs no payload load, a two-material palette no header load.
- The GPU brick pool holds about 18% more bricks for the same bytes.
- The summaries branch of decision 0024 (`NodeView::brick_counts`, not on
  main yet) reads the tree's brick storage and needs this layout when it
  merges: a leaf in `A` and not `B` counts its mask's population for the
  default, and a two-material block has no header.
- Next, by size (same chunks): a default for bricks without full leaves
  (above, 2.7 MiB CPU and 3.3 MiB GPU) once edits there are cheap; the 2.0
  million other full leaves as 16-bit halves (4 MiB, 0021); the GPU's
  second mask only for bricks that need it (a flag in header word 1;
  185,384 bricks, about 1.4 MiB); and the 8.5 million words of one-bit
  indices themselves, which a leaf whose two materials split along an axis
  could replace by a plane.
