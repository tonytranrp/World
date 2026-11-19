# World_Core Benchmarks

## Purpose

This folder holds the Criterion benchmarks for World_Core. They exist to catch performance regressions in the structures that sit on every frame's critical path, and to give the budget numbers needed to hold 60 fps on the performance-floor laptop. CI compares results against a stored threshold and fails on a regression beyond it.

## Scope

- Tree edit cost: single-voxel writes, box fills and carves, and large batches, across empty, sparse and dense regions, reporting cost per edit and per touched brick.
- Traversal cost: hierarchical ray traversal for primary-style rays, grazing rays and long empty rays, and the mip-limited variants used for distant queries.
- Bulk construction from dense input and from generator output.
- Mip refresh cost after typical edits.
- Serialization and deserialization throughput of overlays and subtrees.
- Allocation behavior: growth under long edit sessions, compaction time, and memory per voxel at representative densities.

## Out of scope

- Correctness checks: the tests folder.
- GPU timings, frame-time budgets and brick streaming: World_Render, which uses GPU timestamp queries and Tracy.
- Worldgen throughput: benchmarks in World_Gen.
- Physics step cost: World_Physics.

## Relationships

Benchmarks use only the public API plus dev-only dependencies, and reuse generators of representative scenes so results are comparable across runs. Their results feed the performance gate in CI.

## Rules

- Inputs are generated from fixed seeds so runs are comparable.
- Each benchmark measures one thing and is named for it; setup is excluded from timed regions.
- Benchmarks reflect real use: realistic edit shapes and ray distributions, not synthetic best cases.
- Results are interpreted against the Mac-class CPU budget, not only the development machine.
