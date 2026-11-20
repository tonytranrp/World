# Format

## Purpose

Defines the GPU tree format of decision 0007: the 16-byte node record and the variable-size brick block that the traversal shaders walk, and the pure transcoding from any World_Core `VoxelSource` into them. The CPU tree keeps its edit-friendly layout; this folder is the single place where its content becomes GPU bytes.

## Scope

- The format constants (node info bits, node kinds, brick reference packing, block header fields, payload and palette encodings, brick table and request layouts), in one Rust module that the WESL module `assets/shaders/common/format.wesl` mirrors.
- The `Pod` node record and its constructors for every kind: internal, brick, not resident, uniform.
- The brick block encoder: one block per level-1 node, leaf masks first, payload words next, palette blocks last.
- The representative material of a node, defined exactly and deterministically.
- A CPU decoder of the GPU format, used by tests, benchmarks and pool consistency checks, never by the frame.

## Out of scope

- Deciding what is resident, allocating space and recording uploads: `stream` and `pool`.
- Reading the CPU tree's storage layout: only the public read interface (`VoxelSource`, `NodeView`) is used.
- Shader code: `assets/shaders`.

## Relationships

Depends on World_Core's public read interface and constants. `stream` encodes through it, `pool` sizes from it, the traversal and brick-cache shaders mirror its constants, and the tests decode through it.

## Rules

- Encoding is pure and deterministic: the same source content gives the same bytes on every machine.
- Every constant the GPU reads is defined here once and mirrored into WESL; a test parses the shader text and fails on drift, until Xtask generates the mirror.
- Every block and node is 16-byte aligned and never straddles a 16-byte boundary across records.
- The decoder never panics on bad data: corrupt references are reported as errors, so read-back pools can be validated safely.
