# 0034: Rays jump clear 2x2x2 blocks of cells

Status: accepted, 2026-11-21

## Context

After the beam prepass (decisions 0023 and 0026) the trace is still the
largest GPU pass on the Windows laptop (RX 6800M, Vulkan): 1.5 to 2.0 ms
on the floor walk and 3.4 to 4.3 ms on the default profile's walk
(`docs/research/perf-baseline-windows-render.md`). Decision 0015's
whole-node skip jumps a ray to a node's exit when the box of cells between
it and the exit holds no set bit; when that box is not clear (a surface
ahead in the same node, the common case near terrain) the ray crosses the
node's empty cells one at a time, up to ten steps per node at each level.
The 64-tree post by dubiousconst282 skips "coarse empty" 2x2x2 groups of
cells inside a node in that case.

## Decision

`traversal/tree.wesl`: in an empty cell (of a node at any level, or a voxel
of a leaf mask), if the whole-node skip does not apply, the ray tests the
aligned 2x2x2 block of cells around its cell against the mask
(`half_node_empty`: one word of the mask and two shifted masks) and, when
no bit is set, jumps to the block's exit. The jump code takes the region's
size as a power of two (`jump`, in bits of voxels): `2 level` for a cell,
`2 level + 1` for a block, `2 level + 2` for the rest of a node.

Why the hit is unchanged:

- Blocks aligned to two cells in the node stay aligned in the traversal's
  mirrored frame (the tree's size is a multiple of every block), so the
  block's exit is found by the same exact boundary arithmetic as a cell's,
  and the ray lands on the voxel the reference DDA would enter there.
- The skipped cells have clear bits: no stop, request or brick stamp
  happens in them.
- The ancestor stack restarts at the same level as after a single cell's
  jump from the same node (`jump / 2`), so the footprint test runs at the
  same levels where the ray meets something.

## Measured

Bench (`WORLD_TEST_ADAPTER=gpu cargo bench -p world_render --bench
gpu_timing`, RX 6800M, timestamps, medians of 20), trace alone, before
(commit 129220b) and after:

| Scene | Config | Before | After | Change |
|---|---|---|---|---|
| hills | no beam | 2.55 | 2.26 | -11.5% |
| hills | 64/11 (default) | 1.32 | 1.17 | -11% |
| horizon | no beam | 2.41 | 2.17 | -10% |
| horizon | 64/11 (default) | 1.08 | 0.99 | -8.5% |

Beam and trace in one submission (wall): hills 2.31-2.35 to 2.18 ms,
horizon 2.12 to 2.04 ms.

Alternating walks (`--exit-after-frames 3000 --walk 8 --backend vulkan`),
trace pass at frames 1000 and 2000 (timestamps, ms mean):

| Walk | Before | After |
|---|---|---|
| floor 1 | 1.60, 1.76 | 1.53, 1.69 |
| floor 2 | 1.63, 1.83 | 1.50, 1.77 |
| default 1 | 3.54, 3.94 | (5.40, disturbed), 3.77 |
| default 2 | 3.50, 4.01 | 3.38, 3.82 |

4 to 8% less trace time in game, 8 to 12% in the bench; the beam, shade
and composite passes are unchanged. Frame times moved within noise
(floor steady mean 6.4 and 7.2 ms before, 6.4 and 6.7 after; default 9.1
and 9.6 before, 9.3 after in the undisturbed run).

All 27 GPU tests (with `gpu_beam`'s bit-for-bit comparisons with and
without the beam at every footprint bias, and `gpu_trace`'s CPU
comparisons) and the 24 golden images pass unchanged.

## Alternatives considered (measured and dropped)

- **A beam with eight lanes per tile**, each lane looking up one of the
  cells the beam's cube overlaps, with the tile's lanes combining their
  results through workgroup memory each step (exact: `gpu_beam` passed).
  Bench: the prepass alone 15 to 17% cheaper without head rows and 42%
  cheaper at 256 steps, but with the head rows of decision 0026 it was 8
  to 20% slower, and in game the beam pass rose from 2.0-2.3 to 3.9-4.3 ms
  at the default profile's resolution. The eight times as many workgroups
  crowd out the head rows' trace and pay two barriers per step.
- **The same lookups in one thread, a level per round** (so a round's node
  loads are independent): the prepass doubled (0.84 to 1.61 ms on `hills`),
  most likely from the per-cell arrays spilling out of registers.
- **No head rows** (`beam_head_share: 0`, decision 0026's split): in game
  the beam pass took the same 2.2 ms with and without them and the trace
  grew 7 to 15%, so the head rows are free on this GPU and stay.

## Consequences

- One more mask test per empty cell where the whole-node skip fails.
- Next for the trace and the beam: the in-game beam (2 ms) is far more
  than the bench's (0.85 ms): its marches are longer on real terrain,
  most likely grazing tiles that reach the 64-step cap. A cheaper sky
  proof (decision 0026's per-column height summary) or a coarser first
  level for such tiles is the next thing to measure.
