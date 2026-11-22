# 0033: Full leaves of other materials are 16-bit halves

Status: accepted, 2026-11-21

## Context

Since decision 0021 a full one-material leaf that is not its brick's
default material is one word in the brick's block: `UNIFORM_BIT` and the
material. Material ids are 16 bits, so half of every such word is the flag,
which the full mask already says. Decisions 0021 and 0025 listed them as
16-bit halves for later (4 MiB). After decision 0031, on the 107
full-detail chunks: 2,011,900 such leaves in 131,889 bricks, 7.7 MiB of
145.5 MiB live.

## Decision

In a brick whose header has the full mask, the materials of the full
leaves of other materials, `O = B & !A`, come right after the header as
16-bit halves in slot order, two a word, low half first; an odd count
leaves the last high half zero. The other stored leaves follow, so stored
leaf `i` (outside `B`) starts at `header + ceil(|O| / 2) + 2 * rank(mask &
!B, i) + rank(mask & !A & !B, i) + 2 * rank(P, i)`, and the material of a
leaf in `O` is half `rank(O, i)`. Bricks without the full mask, and so
without such leaves, do not change.

Readers: `BrickHeader::half` and `leaves_start`, the full-mask path of
`step` (one half read instead of one word), the in-order walk of
`NodeView::children` and `brick_counts` (the halves first, then the other
leaves). `MemoryBreakdown` gains `full_words`, the words holding the
halves.

## Alternatives considered

- **Halves interleaved with the other leaves in slot order**: a leaf's
  offset would need half-word addressing for every leaf after it.
- **One shared material for all other full leaves of a brick** (most such
  bricks have one other material): a header word instead of a half a leaf,
  but bricks with several others need the halves anyway.

## Consequences

`cargo test --release -p world_gen --test chunk_bytes -- --ignored
--nocapture`, before (3d28c70) and after:

| 107 full-detail chunks | Before | After | Change |
|---|---|---|---|
| CPU tree, live | 145.5 MiB | 141.7 MiB | -3.8 MiB, -2.6% |
| CPU tree, pages held | 145.6 MiB | 141.9 MiB | -2.5% |
| Chunk median (p10, p90) | 1,374 KiB (638, 2,148) | 1,333 KiB (638, 2,118) | |
| Words for other full leaves | 2,011,900 | 1,020,671 | |

Times, on prebuilt executables of 3d28c70 and this change, alternating,
three rounds (1 s warm-up, 3 s measurement). The first two rounds ran
while other builds loaded the machine unevenly (the same executable moved
up to 60% between them); the third ran on a quiet machine and is the one
given:

| Bench (third round) | Before | After | Change |
|---|---|---|---|
| chunk_reads/get_100k_near_surface | 7.12 ms | 7.01 ms | -2% |
| chunk_reads/cursor_100k_near_surface | 7.84 ms | 7.84 ms | 0 |
| chunk_reads/cursor_rays_256 | 184 µs | 176 µs | -4% |
| chunk_reads/walk_every_brick | 173 ms | 170 ms | -2% |
| chunk_edits/crater_r12_and_restore | 497 µs | 472 µs | -5% |
| chunk_edits/scattered_1k_and_restore | 1.13 ms | 1.15 ms | +2% |
| apply_1k/clustered_16, scattered_128 | 49.1, 359 µs | 47.6, 367 µs | -3%, +2% |
| coherent_reads/bottom_up line, brick, scattered | 14.3, 165, 576 µs | 13.9, 166, 581 µs | -3%, +1%, +1% |
| serialize/decode_terrain_512 | 12.3 ms | 12.7 ms | +3% |
| serialize/encode_terrain_512 | 12.6 ms | 14.4 ms | +14% (one sample; -9% over all three rounds) |
| terrain_tree/chunk_build_only | 18.1 ms | 17.6 ms | -3% |

No change is resolved beyond this machine's noise; the read paths do one
shift more for an other full leaf and one popcount of `O` more for the
leaves after the halves in a brick with the full mask.

- The GPU block format, the encoder and saves do not change (the encoder
  and serialization read through `NodeView`).
- The instruction-count gate (gungraun, Linux) was not run.
