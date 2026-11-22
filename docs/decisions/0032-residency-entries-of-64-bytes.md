# 0032: Residency entries shrink from 112 to 64 bytes

Status: accepted, 2026-11-21

## Context

The brick cache's CPU residency table (`brick_cache/stream/residency.rs`)
holds one `Entry` per node with a GPU record: 1.2 to 1.5 million on the
floor walk, 2.0 million on the default profile's walk (render baseline,
`docs/research/perf-baseline-windows-render.md`). Each was 112 bytes, plus
about 18 bytes of table (decision 0014):

| Field | Bytes | Used by |
|---|---|---|
| `record: GpuNode` | 16, aligned to 16 | every entry |
| five child masks (`kids`, `layout`, `bricks`, `grown`, `done`) | 40 | `layout` every entry, the other four only nodes with children entries |
| `brick: Option<BrickSlot>` | 28 | level-1 entries |
| `group: Option<NodeAlloc>` | 20 | nodes with children entries |
| `index`, `last_requested` | 8 | every entry |

Level-1 entries, the large majority, never have children entries (their
children are leaves, which have no records), so 60 bytes of each were
always empty, and `GpuNode`'s 16-byte alignment padded the entry.

## Decision

- **`Below`**: the group and the four child masks move into a boxed struct
  (48 bytes), allocated the first time anything sets them; `None` reads as
  all empty. Accessors (`group()`, `kids()`, `below()`, `below_mut()`) keep
  every call site's meaning.
- **`BrickSlot`** stores its id plus one in a `NonZeroU32`, so
  `Option<BrickSlot>` has no tag (24 bytes).
- **`GpuNode`** is `repr(C)` with word alignment. The pool's records are 16
  bytes apart in a buffer aligned by wgpu, so each is still 16-byte aligned
  on the GPU; only CPU structures that embed a record change.

`Entry` is now 64 bytes (`layout`, record, `index`, `last_requested`,
brick slot, `Below` pointer), held by a const assertion. Entries are no
longer `Copy`; the four places that copied one now clone or borrow.

## Measured

Sizes from the types (rustc 1.97.1, x86_64): `Entry` 112 to 64 bytes,
`Below` 48 bytes on the heap for nodes with children entries (about 6% of
entries: 73,000 to 122,000 expanded nodes on the walks).

Alternating walks (`--exit-after-frames 3000 --walk 8 --backend vulkan`,
release), the previous commit (base) and this change (new), peak private
bytes of the process sampled once a second:

| Walk | Records at frame 2000 | Peak private, base | Peak private, new |
|---|---|---|---|
| floor, pair 1 | 1.25 million | 2,651 MiB | 2,584 MiB |
| floor, pair 2 | 1.24-1.25 million | 2,655 MiB | 2,557 MiB |
| default | 2.0 million | 4,210 MiB | 4,083 MiB |

67 to 127 MiB less, about the expected 48 bytes per level-1 entry less the
`Below` boxes (1.25 million x 48 bytes is 57 MiB, 2.0 million is 92 MiB;
the peak also depends on the walk). GPU pass times and frame times did not
change (floor steady mean 6.1-6.3 ms base, 6.3-6.4 new; default 8.4 and 8.5).

CPU time (`cargo bench -p world_render --bench encode`, against a baseline
saved before the render work; the run in between also carries decisions
0028 and 0030, which do not touch these paths):

| Bench | Base | New | Change |
|---|---|---|---|
| `residency/make_resident/terrain` | 7.37 ms | 6.79 ms | -8.5% |
| `residency/expand_churn/terrain` | 203 us | 177 us | -11.9% |
| `load_bricks/terrain_1_threads` | 3.10 ms | 2.63 ms | -16.1% |
| `load_bricks/terrain_8_threads` | 1.68 ms | 1.50 ms | -11.7% |
| `changes/surface_chunk` | 789 us | 667 us | -16.8% |
| `changes/coarse_chunk` | 239 us | 176 us | -25.6% |
| `changes/coarse_chunk_summaries_on_worker` | 191 us | 143 us | -20.2% |

Smaller entries are cheaper to shift when the table's sparse arrays insert
or remove (decision 0014) and to bring through the cache; the extra
indirection for `Below` costs less than that.

## Alternatives considered

- **Two tables**, one of 48-byte level-1 entries and one of full entries
  for levels 2 and up: 16 bytes less per level-1 entry and no heap boxes,
  but every lookup would choose a table by level and most of the file's
  operations would change. The box keeps the change local.
- **Dropping `index`** (derivable from the parent's group and the rank of
  the slot in its layout): another 4 bytes, for a parent lookup on every
  record write.
- **Shrinking `BrickAlloc`** (its `BrickRef` duplicates the allocation's
  offset): 8 bytes more per resident brick only.

## Consequences

- About 50 to 130 MiB less CPU memory on the walks, and the residency
  benches 8 to 26% faster.
- A `Below` is allocated for any node that once had children entries and is
  kept until the entry goes; collapsed nodes keep theirs (48 bytes each).
