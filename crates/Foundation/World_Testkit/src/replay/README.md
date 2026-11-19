# Replay

## Purpose

This module defines what a deterministic replay is and how to check one. A replay record names a scenario, says how many steps it ran, and lists one hash per step. A scenario is deterministic if two runs give identical hash lists, and unchanged if its hashes equal the ones recorded on disk.

## Scope

- The record: scenario name, step count and per-step hashes.
- Reading and writing records as stable text with a format version, fixed-width hex and fixed field order, so review diffs are readable and the bytes are reproducible.
- Running a scenario twice, step by step, and reporting the first step at which the runs differ.
- Verifying a run against a recorded replay and reporting the first differing step with both hashes.
- Recording a new replay only on explicit request, through the same update workflow as other golden files.

## Out of scope

- The scenarios. The crates under test supply the step function and this module drives it.
- The recorded files and where they live: the `tests/replays` folder and the root tests crate.
- Stepping an ECS, physics or schedule randomization: World_Testkit_Bevy.
- Computing the hash of a state: hash.

## Relationships

Builds on hash for the values and on golden for the update workflow. Scenarios are plain closures or a small trait, so no engine type is required. Used by crate-level determinism tests and by the replay suites in the root `tests/` folder.

## Rules

- A replay is recorded only from a run that already matched itself on repeated execution.
- The format is versioned, and an unknown version is an error, not a guess.
- Step count is part of the record. Runs of different length never compare equal.
- No wall-clock input and no dependence on thread count.
