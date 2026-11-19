# Hash

## Purpose

This module gives the project one way to turn state into a number that means the same thing everywhere. Replays, worldgen golden hashes and canonical-tree comparisons depend on hashes that are identical on Windows and macOS, on Intel and Apple Silicon, and across library upgrades.

## Scope

- xxh3-64 through the `xxhash-rust` crate, chosen because its output is frozen across versions. The standard library's default hasher is deliberately not used.
- A trait for types that feed their state into a hasher in a defined encoding: fixed-width integers in little-endian order, floats by exact bit pattern, a length written before every sequence, and no dependence on pointer width, padding or memory layout.
- Implementations for primitives, slices, tuples and the kit's own types.
- Formatting a hash as fixed-width lowercase hex for use in text records.

## Out of scope

- Cryptographic hashing or integrity checking.
- Deciding which state of a production type is hashed: the test that hashes it chooses, using the trait defined here.
- Record layout and files: replay and golden.

## Relationships

A leaf module. Used by replay for per-step hashes, by golden for text and hash comparisons, and by tests that compare canonical trees by content hash.

## Rules

- A recorded hash never changes meaning. The encoding is fixed, and changing it is a recorded decision, not an edit.
- No platform-dependent input: no `usize`, no hash-map iteration order, no uninitialized padding.
- Floats are hashed by exact bits, so a one-ulp difference changes the hash. That is intended.
- The same input gives the same hash on every machine.
