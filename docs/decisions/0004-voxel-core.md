# 0004: Voxel core

Status: accepted, 2026-11-19. The leaf nodes in "Node layout" and the flat
page list in "Storage and snapshots" are superseded by decision 0005 (leaves
inline in their level-1 parent, a two-level page directory). Decision 0006
replaces the assumption that the whole world is resident with chunk tiers.

## Context

The voxel core is the authoritative CPU world in `World_Core`: the sparse
64-tree, its materials, edits and serialization. The plan's done criterion is
one million random edits with no divergence from the dense reference model.
Decision 0003 requires that the renderer and brick streaming read an immutable
snapshot taken once per frame, with no locks, and that old versions are freed
when no snapshot uses them. The folder READMEs fix the shape: a 64-bit child
mask per node, children packed by popcount, palette-compressed leaves, indices
instead of pointers, and canonical trees.

## Decision

### World and addressing

- The tree has `TREE_DEPTH = 10` node levels. A level-`L` node has 4x4x4
  children that each cover `4^L` voxels per axis, so level-0 nodes are leaves
  whose children are single voxels, and the root (level 9) covers
  `4^10 = 1,048,576` voxels, 16.384 km, per axis.
- The world is the cube `[-524,288, 524,288)` voxels on every axis, centred on
  the origin so f32 metres stay precise to about 1 mm at the edge. Reads
  outside it return air; writes outside it are an error.
- Inside a node, child slot `i = x + 4y + 16z`, where `x`, `y`, `z` are the
  two bits of the world-local coordinate (`pos + 524,288`) at bit `2L`.
- A **brick** is the 16x16x16-voxel region of one level-1 node (25 cm). It is
  the unit of change reports and, later, of GPU streaming. A **leaf** is the
  4x4x4 level-0 node.

### Node layout

- Every node is 16 bytes: `mask: u64`, `base: u32`, `info: u32`. The level,
  known during traversal, decides how the node is read.
- **Branch** (level 1 and up): mask bit `i` is set when child `i` is not all
  air. The children are packed at `words[base..base + popcount(mask)]` in slot
  order. A child word with the top bit set is a uniform child whose material id
  is in the low 16 bits; otherwise it is a node index.
- **Leaf** (level 0): mask bit `i` is set when voxel `i` is solid. `info` holds
  the palette length (low byte) and the index bit width (second byte). At
  `words[base..]` are the palette, two u16 material ids per word (low half
  first), then one index per solid voxel, in slot order, packed LSB-first into
  whole words. The bit width is the smallest of 0, 1, 2, 4 or 8 that can index
  the palette; with one material it is 0 and no index words follow.

### Canonical form

The same voxel content always produces the same structure:

- An all-air child is a clear mask bit and has no word.
- A subtree that is all one solid material is a uniform child, never a node.
- A leaf's palette holds exactly the materials present, sorted by id.
- The tree's root is itself a child value: air, uniform or a node.

So two trees with equal content have equal masks, palettes and uniform
children everywhere, and content equality is a structural walk. Node indices
and word offsets are not part of the content and may differ.

### Storage and snapshots

- Nodes and words live in two arenas of fixed-size pages, each page behind an
  `Arc`, with the page list itself behind an `Arc`. Freed node slots go on a
  LIFO free list; word blocks (1 to 64 words) are recycled by exact size and
  never straddle a page.
- `VoxelTree::snapshot` clones the two page-list `Arc`s and the root: O(1). The
  writer changes a page only through `Arc::make_mut`, so the first write to a
  page that a snapshot still shares copies that page and the snapshot keeps the
  old one. A snapshot therefore never changes, needs no locks, is `Send + Sync`,
  and its pages are freed when the last snapshot holding them is dropped.
- This page-level copy-on-write replaces per-node path copying: it gives
  readers the same guarantees with far fewer allocations, and lets the writer
  update a node in place when no snapshot shares its page. Each edit still
  touches only the nodes on its path, so its cost is bounded by the depth.

### Edits

- `set` changes one voxel and reports whether anything changed. `fill_box`
  sets every voxel of a box, replacing whole covered subtrees with uniform
  children. `apply` runs a batch of edits after checking every position, so a
  batch either applies completely or not at all, and returns the sorted set of
  bricks whose content changed. An edit that changes nothing reports nothing.
- After every write the tree is canonical: emptied children are removed, and
  uniform subtrees collapse, freeing their nodes and words.

### Serialization

- A tree is encoded depth-first from the root: one tag per child (air, uniform
  with its u16 id, branch with its mask, leaf with its mask and palette and
  packed indices), little-endian, after a header with a magic number, the
  format version, the tree depth and the body length, and followed by an xxh3
  checksum of the body.
- Because the tree is canonical, equal content gives identical bytes, and the
  xxh3 of the body is the content hash used by golden tests.
- Decoding checks every length, mask, palette and index against the format and
  rebuilds a canonical tree, or returns an error. It never panics and never
  allocates more than the input can justify.

### Deferred

Coarse mip summaries arrive with the GPU tracer, the per-chunk edit overlay
with World_Gen, the change journal and compaction with the brick cache, and
CPU ray traversal with the CPU reference tracer. None of them change the
layout above.

## Consequences

- World_Core gains a dependency on `xxhash-rust` for checksums and content
  hashes.
- The tree, alloc and mutate READMEs now describe copy-on-write pages where
  they described path copying.
- The node layout is a format: changing it means a new serialization version
  and matching WESL changes once the GPU mirror exists.
