# Tree Nodes

## Purpose

This module defines the in-memory shape of one tree node and one leaf brick. The node layout is the contract between the CPU tree and the GPU traversal code, so it is specified here precisely and kept small, aligned and cache friendly.

## Scope

- The interior node: a 64-bit child mask with one bit per 4x4x4 child, and a densely packed child array indexed by the population count of the mask bits below the child's bit.
- The leaf brick: a small dense block of palette indices, with its palette of u16 material ids and the bit width of an index.
- The uniform and empty representations, where a whole subtree is a single material or nothing and carries no children.
- Bit-level helpers: child bit selection, popcount indexing, mask iteration, and conversion between child index and 3D offset.
- Size and alignment guarantees, and compile-time assertions on them.
- Encode and decode of palette-compressed index arrays at each supported bit width.

## Out of scope

- Allocating or addressing nodes in memory: alloc.
- Rewriting nodes and splitting or merging them: mutate.
- Coarse-level summaries: mip.
- GPU buffer packing and upload ordering: World_Render. This module defines the logical layout; World_Render mirrors it.
- Material meaning and tables: the material module.

## Relationships

It depends on coordinates and on palette types from the material module. Alloc stores these types, mutate rewrites them, mip annotates them, and the ray module and serialization read them.

## Rules

- Layout facts (sizes, offsets, mask width, fan-out) are derived from the shared constants and asserted at compile time.
- Nodes hold no pointers and no padding that carries meaning.
- Child-index math uses only integer operations and is bit-exact with the WESL implementation.
- Decoding a brick always returns exactly what was encoded.
- Changing a layout is a format-level change and requires matching updates to serialization and the shader mirror.
