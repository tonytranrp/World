# World_Testkit Benchmarks

## Purpose

This folder holds the Criterion benchmarks of the reference model and of the testkit's hot paths. They show what the oracle costs, so the full profile's million random edits stays affordable, and they are the baseline the performance gate in Xtask compares against.

## Scope

- Dense model cost: single edits, box fills and carves, and whole edit sequences at the full profile's length.
- DDA ray cast over empty, sparse and dense volumes.
- Hashing throughput through the hashing trait.
- Divergence checking of a store against the model.
- Replay record reading and writing, and the hit comparator over large ray batches.
- Strategy generation cost per case, since generation dominates some property runs.

## Out of scope

- Performance of the production 64-tree and its traversal: World_Core benches.
- GPU timings and frame budgets: World_Render.
- Correctness checks: the tests folder.

## Relationships

Benchmarks use only the public API plus Criterion. Xtask bench saves their results as a baseline and later compares a run against it.

## Rules

- Inputs come from fixed seeds so runs are comparable. Setup stays outside the timed region.
- Each benchmark measures one thing and is named for it.
- Results are compared only against a baseline from the same machine class.
- A slower oracle is a regression like any other, because it slows every suite that uses it.
- Speed work on the oracle must not cost clarity. Readability wins over throughput.
