# World_Core Tests

## Purpose

This folder holds the integration and property tests that treat World_Core as a black box through its public API. These tests are the safety net for the data structure every other crate trusts.

## Scope

- Property tests using proptest that apply random edit sequences to the tree and to a plain dense array, then assert the two never diverge. This covers single edits, box fills, carves, bulk builds and very large sequences of random edits.
- Canonical-form tests: equal voxel content must produce equal structure and equal content hashes regardless of edit order.
- Serialization round-trip tests, including overlays, version migration, corrupted and truncated input, and byte-for-byte determinism.
- Ray tests that compare hierarchical traversal with the brute-force dense DDA oracle over random trees and random rays, including rays that start inside solids, graze voxel edges or leave the world.
- Mip tests that compare incremental refresh with a full rebuild.
- Coordinate tests for negative values, boundaries and conversion round trips.

## Out of scope

- Unit tests of private helpers: those sit next to the code in the source folder.
- Throughput measurements: the benches folder.
- Tests that need Bevy, a GPU or a window: Engine crates.
- Worldgen determinism and reference-renderer comparisons: tests in World_Gen and World_Cpu_Ref.

## Relationships

Tests use only the public API of the crate and dev-only dependencies. The reference implementations (dense array, brute-force DDA) are written here or in the public oracle code and are intentionally simpler than the code under test.

## Rules

- Every test is reproducible: seeds are fixed or printed, and failing cases are shrunk and kept as regression inputs.
- The dense array is the source of truth for correctness; the tree must match it exactly.
- Tests run quickly enough for every push; the heaviest random runs are bounded by case counts, not wall time.
- No test depends on thread count, platform or iteration order.
