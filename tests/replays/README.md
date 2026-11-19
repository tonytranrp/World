# Replays

## Purpose

This folder holds recorded deterministic replays and the expected results that go with them. A replay is a scene plus the exact sequence of inputs and steps applied to it. The physics determinism check runs a scenario twice, hashes every body's transform at each step, and requires the hashes to match each other and the recorded values. This catches ordering bugs, accidental nondeterminism and silent behavior changes.

## Scope

- Recorded input sequences (player actions, destruction events, tool use) with fixed step counts at the fixed 60 Hz physics rate.
- Expected per-step transform hashes for each replay.
- Regression replays for notable behaviors: the tower collapse, wall breaking into islands, debris settling and sleeping, character controller stepping over rough voxel ground.
- Replays used to exercise schedule-order randomization, so hidden system-ordering assumptions show up as hash differences.
- Metadata: which scene it starts from, the engine and physics settings it assumes and the thread count.

## Out of scope

- The starting scenes. Those are in `../scenes`.
- Reference images. Those are in `../golden`.
- Gameplay recordings for demonstration, bug reports from players or telemetry; replays here exist only as tests.
- Replay recording and playback code, which lives in the crates and in `tools/Xtask`.

## Relationships

- Run by integration tests in the parent `tests/` folder against `World_Physics`, `World_Core` and `World_Sim`.
- Start from scenes in `../scenes`.
- Executed in CI by workflows in `.github/workflows`.

## Rules

- A replay is reproducible: fixed seeds, fixed step count, no wall-clock input.
- Hashes are recorded only from runs that matched across repeated executions.
- A hash change is accepted only with an explanation: an intended physics change, a library upgrade, or a fixed bug. Re-recording is a deliberate, reviewed step.
- Data format is versioned by the code that reads it; old replays are migrated or removed, not left broken.
- Binary data is stored through Git LFS.
