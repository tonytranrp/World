# 0035: Edits splice bricks and sort by radix; wider palettes share the mixed mask

Status: accepted, 2026-11-21

## Context

Two things measured against main (2d2f604, decisions up to 0029) after
decisions 0031 and 0033 (inline two-material leaves, other full leaves as
halves):

- CI's instruction-count gate (gungraun, Linux, 5% limit) failed on PR #10:
  `get_10k/terrain` +6.3%, `apply_1k/dense_block` +5.2%,
  `snapshot_then_edit/striped_128` +12.9%, and `set_1k` +4.4% (empty tree)
  and +3.3% (dense block). The lead reverted 0031 and 0033 on the
  integration branch until they are fixed.
- 0031 noted scattered single-voxel edits 5 to 19% slower: a leaf that
  changes between one and two materials changes its brick's length, so the
  whole brick was decoded into 64 records and packed again.

And this is a destruction game, so the edit path was measured on the
pattern players make: craters (one `apply` of every voxel of a ball,
radius 16 to 128 voxels, a quarter metre to two metres) and a tunnel of 24
digs. On real generated chunks (new `destruction` group in World_Gen's
`chunk_reads` bench), main spent about 100 ns a voxel: 1.1 s for a 2 m
crater. Timers in `apply` on the 1 m crater (1.1 million edits) gave:
computing the tree-order keys 20 ms, the stable sort 85 ms, dedup and the
residency check 14 ms, the tree writes 30 ms. Four fifths of a crater was
sorting the edit list, not changing the tree.

## Decision

**Sorting.** `apply` orders its batch with `tree_order`: each edit's
tree-order key comes from bit spreading (`node::world_key`, four
shift-and-mask stages per axis instead of a loop over ten levels), and a
batch of 256 or more edits is sorted by a stable least-significant-digit
radix sort over only the key bits that differ within the batch (a batch's
keys share every higher bit), with the material packed into the same
`u64`; dedup keeps the last edit of each voxel as before. Batches spread
over more than 4 km, whose differing bits do not fit, and short ones keep
the comparison sort.

**Whole children.** A run of edits that names every voxel of a child of
level 1 or more with one material (a crater's inside) replaces the child
whole: its subtree is freed and it becomes a filled child, without a write
per voxel. The changed bricks are reported from the old subtree. The
internal change callback now promises at least one edit of every changed
brick, not every changed voxel with its old material (no caller used
those).

**Splicing bricks.** A leaf that cannot be rewritten in place no longer
decodes its brick: `Writer::splice` copies the stored block with the
changed leaves' words replaced, as long as the default material stays (no
full leaf of the default went, none of another material came, or all full
leaves are of the default) and the node stays a node; otherwise the brick
repacks as before. Leaves that keep their length are still patched in
place as they are found; up to eight others wait for one splice at the end
of the brick's edits, more repack. In debug builds every splice is checked
equal to the repack.

**Format (CPU tree only).** Two changes to 0031 and 0033's layout:

- Leaves of three or more materials join the pair mask, now the *mixed
  mask*: five words, the leaf mask, the palette block's offset and two zero
  words (a two-material leaf's index words are never zero, which tells the
  two apart). A leaf that gains or loses a material then keeps its length
  and is patched in place, as before 0031. Real terrain has no such leaves
  (the census below), so no bytes change there.
- The halves of other full leaves end the block instead of following the
  header, so a stored leaf's offset no longer needs the popcount of the
  other full leaves; a half is found from the block length in `info`.

**Reads and offsets.** `brick_step` (bricks with a full mask) ranks the
mixed mask only when the brick has one. `BrickHeader::record_offset` takes
one rank for bricks without a full mask (two with a mixed mask) instead of
three or four.

**Allocator.** The word allocator's free lists are indexed by length again
(0031 had made them a sorted vector searched on every allocation and free),
grown only to the longest freed length.

## Alternatives considered

- **Leaving slack after each brick** so a longer leaf fits in place: bytes
  in every brick for edits most bricks never see.
- **Making every partial leaf five words** so single and two-material
  leaves trade places in place: 6.8 MiB on the 107 chunks, most of 0031's
  saving.
- **Radix digits of 11 bits** (three passes for a 1 m crater instead of
  five): the scatter into 2,048 buckets ran no faster on this machine
  (18 to 30 ms against 20 to 21 ms for 8-bit digits, loaded), so 8 bits.
- **Deferring every repack to the end of the batch**: `apply` already
  rebuilds each brick once per batch; the cost was the repack itself.

## Consequences

**Bytes.** `cargo test --release -p world_gen --test chunk_bytes --
--ignored --nocapture`: the 107 full-detail chunks are 141.7 MiB live,
141.9 MiB held (median 1,333 KiB a chunk), exactly 0033's figures: they
hold no leaf of three or more materials (`palette_records: 0`), and moving
the halves moves no word. A synthetic block of random materials (the
`dense` scenes) pays two words more per leaf.

**Destruction**, `cargo bench -p world_gen --bench chunk_reads --
destruction chunk_edits`, prebuilt executables of main (2d2f604), 0033
(893f8fb) and this change run alternately, two rounds (1 s warm-up, 3 s
measurement; the machine shared with two other builds). Medians of the two
rounds, and nanoseconds per voxel the batch names:

| Bench | main | 0033 | this | vs main | ns/voxel, main to this |
|---|---|---|---|---|---|
| destruction/crater_r16 (17 k voxels) | 0.81 ms | 0.68 ms | 0.47 ms | -42% | 47 to 27 |
| destruction/crater_r32 (137 k) | 10.8 ms | 10.5 ms | 7.2 ms | -34% | 79 to 52 |
| destruction/crater_r64 (1.1 M) | 103 ms | 101 ms | 60.6 ms | -41% | 94 to 55 |
| destruction/crater_r128 (8.8 M) | 954 ms | 973 ms | 450 ms | -53% | 109 to 51 |
| destruction/tunnel_24_digs_r24 (1.4 M) | 78.5 ms | 88.9 ms | 48.6 ms | -38% | 56 to 35 |
| chunk_edits/crater_r12_and_restore | 581 µs | 544 µs | 440 µs | -24% | |
| chunk_edits/scattered_1k_and_restore | 1.43 ms | 1.35 ms | 1.20 ms | -16% | |

A dig of the tunnel (a 0.375 m ball, 58 k voxels) takes 2.0 ms instead of
3.3 to 3.7 ms. The scattered edits, which 0031 had made 5 to 19% slower,
are 16% faster than main and 11% faster than 0033.

**The gate's operations.** Valgrind does not run here, so each operation of
`benches/instructions.rs` was timed in one process holding both builds (two
copies of World_Core under different crate names), main and this change
run alternately on fresh inputs, 400 times each (1,200 for the reads),
thread cycles from `QueryThreadCycleTime`; the same harness put main
against 0033 at 0% for `get_10k`, +7 to +9% for `apply_1k` and +12 to +22%
(minimum) for `snapshot_then_edit`, so it sees the direction of the gate's
failures but counts cycles, not instructions:

| Operation, this change against main | min | p10 | p50 |
|---|---|---|---|
| get_10k/terrain | +0.2% | +0.3% | +0.2% |
| apply_1k/dense_block | -5.2% | -2.2% | -1.5% |
| set_1k/dense_block | +2.1% | +4.2% | +8.2% |
| set_1k/empty_tree | +2.1% | +2.4% | +1.3% |
| snapshot_then_edit/striped_128 | +1.4% | -11.5% | -41.8% |
| decode_tree/terrain | -7.0% | -7.6% | -4.7% |

`set_1k/dense_block` writes 1,000 leaves of eight or more materials, now
five words each instead of three, so the block is larger in cache; the
instructions per write are those of main plus one rank. The gate itself
still has to be run (the lead offered a throwaway PR).

- `compare_after_edits` against `DenseWorld`, the property tests, the
  canonical walks (World_Core's and World_Testkit's, now counting every
  leaf of two or more materials as five words) and every splice checked
  against the repack in debug builds pass. New tests: tree order against a
  stable sort with dedup (small, large and world-spread batches, repeats),
  whole bricks and a 64^3 node replaced by covering batches with exactly
  the changed bricks reported, world keys against `path_key` and back.
- GPU: the encoder reads through `NodeView`, so blocks, shaders and saves
  do not change; `WORLD_TEST_REQUIRE_GPU=1 cargo xtask test gpu` passes
  (26 tests), with World_Render's `gpu_trace` world-tree test asserting
  `pair_leaves > 0` (the lead's fix on the integration branch).
- **Gate, first run** (PR #11, 0031 + 0033 + this): 9 of 10 under the
  limit (`apply_1k` -1.9%, `set_1k` +0.6% and +2.1%, `snapshot_then_edit`
  +3.0%, `decode_tree terrain` -3.4%), but `get_10k/terrain` +8.65%. A
  census of its 10,000 probes: 63% end above level 1 (identical code), the
  rest all in bricks with a mixed mask and no full mask (1,847 default
  leaves, 771 two-material, 375 three-material, 83 records, 658 air
  slots); main read those bricks on its one-rank path. The generated code
  of `TreeView::lookup` built a `Step` value for the brick and matched it
  again (an unreachable arm's compare included) and computed the leaf slot
  before knowing it was needed. Point lookups now end in `brick_voxel`,
  which returns the material from every branch, tests the full-mask flag
  first and reads the mixed mask only after the default test (so a default
  leaf costs what it did on main), and a record (now always one material)
  is read without a palette test. `TreeCursor` keeps `step`.
- **Gate, second run**: `get_10k` unchanged (+8.6%). Callgrind's per-file
  breakdown put the excess in the page reader on every probe, and counting
  page-cache misses in the scene (a counter in copies of both builds) gave
  27,816 over the 10,000 probes against main's 22,340. The scene is built
  by box fills, so its blocks sit where the edits left them: the upper
  chunk's level-2 block (41 child words, read by every probe that reaches
  it) had drifted to page 2 or 4, away from its parents on page 0, because
  bricks change length more often since 0031 and every growth moved blocks
  to the arena's end. The word allocator now takes the lowest block of a
  short free list (the last freed of a long one), cuts a request from the
  shortest longer free block when no block of its length is free, and
  grows or shrinks the arena's last block in place. The scene then has
  21,886 misses (main 22,340), 18,937 free words instead of 37,862 (main
  21,919) and 10 pages instead of 15 (main 11). Built chunks, which free
  nothing, do not change.
- Next: big craters are now about half sorting and half tree writes; a
  ball fill that replaces covered nodes without listing their voxels
  (like `fill_box`) would make a crater cost its surface instead of its
  volume, but needs its own edit-log record in World_Game's residency.
