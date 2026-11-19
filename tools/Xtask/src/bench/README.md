# Bench Command

## Purpose

This folder implements the automation command that runs the project's Criterion benchmarks and judges the results against thresholds. It turns benchmark numbers into a pass or fail signal, so performance regressions fail CI the same way test failures do.

## Scope

- Discovering and invoking benchmarks across the workspace, covering voxel tree edits, traversal, worldgen chunk generation, collider building, island detection and other measured hot paths.
- Reading Criterion output and extracting the statistics that matter, such as median and confidence interval.
- Threshold handling: loading recorded baselines and allowed regression margins, comparing the current run, and reporting which benchmarks regressed and by how much.
- Updating baselines on request, as an explicit action rather than an automatic one.
- Output formats suitable for humans and for CI summaries.

## Out of scope

- The benchmarks themselves, which live in the benches folders of the crates they measure.
- GPU frame-time checks and Tracy capture, which are handled by runtime instrumentation and CI jobs outside this command.
- Running unit or property tests.

## Relationships

Called through the Xtask dispatcher. Reads results written by the benches in Foundation and Engine crates. Uses the shared helpers of the Xtask source folder for running subprocesses.

## Rules

- Comparisons are against stored baselines for the same machine class, since absolute numbers differ between machines.
- A regression beyond the margin yields a nonzero exit code and a clear report.
- Baseline updates are explicit and reviewable.
- The command never modifies benchmark code or results of the crates being measured.
