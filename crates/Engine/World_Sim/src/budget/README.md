# budget

## Purpose

This folder is the shared time-slicing mechanism for all simulations. Every voxel simulation is potentially unbounded, so each frame draws from one common allowance, and no simulation can stall the frame however large its active region grows.

## Scope

- The per-frame budget: a cap expressed as a voxel count, wall-clock time, or both, with the allowance adapting to measured frame time and a floor that keeps simulations making progress.
- Allocation across simulations: weighted shares for stress, sand, fire, and fluids, redistribution of unused share, and priority for work near the player.
- Work queues and cursors: a resumable representation of in-progress work, so a simulation can stop mid-pass and continue next frame.
- Accounting: measuring consumed budget, recording starvation (work waiting too long), and exposing counters for tuning and diagnostics.
- Scheduling policy: ordering active regions by distance, age, or urgency, and coalescing duplicate work.
- Configuration: budget values as data, adjustable at runtime and per platform (a low-end Mac runs with a smaller allowance).

## Out of scope

- The rules of any simulation: stress, sand, fire, fluids.
- Physics-side budgets for island scans and collider rebuilds, which belong to World_Physics. This module governs only simulation work.
- Voxel edits themselves: World_Core.

## Relationships

Depends on Bevy time and diagnostics only. Depended on by every simulation module, each of which requests and spends allowance here. Depends on none of them.

## Rules

- A simulation asks for budget before doing work and stops when it is spent; never the reverse.
- Starvation degrades simulation latency, never frame time.
- Allocation is deterministic for a given sequence of inputs, so tests can reproduce a run.
- The module holds no simulation state beyond work cursors.
