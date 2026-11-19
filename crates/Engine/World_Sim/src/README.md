# World_Sim source

## Purpose

This folder is the source root of the simulation crate. It exposes one Plugin and one prelude, and arranges the independent simulation modules so they run in a fixed order and share a single per-frame budget.

## Scope

- The crate's single public Plugin entry point and its prelude, re-exporting configuration, enable switches per simulation, and events other crates may send or observe.
- Schedule wiring: the order in which budget allocation, stress, sand, fire, and fluids run each frame, and the point where their queued voxel edits are flushed through World_Core's edit API.
- Crate-wide shared types used by several simulations: the active-region markers, the set of per-simulation enable flags, and the edit batch type through which simulations submit changes.
- Module declarations for budget, stress, sand, fire, and fluids.

## Out of scope

- Simulation rules and algorithms: each lives in its own module folder.
- Time budgeting logic itself: budget.
- Rigid-body physics: World_Physics.
- Tests: the sibling tests folder.

## Relationships

Depends on all its module folders, World_Core, World_Physics, and Bevy. Depended on, through the prelude, by World_Game. The simulation modules depend on budget and on shared types here, but not on each other. Interactions such as fire burning a support or sand burying a structure happen through the voxel data and material table, not through direct calls.

## Rules

- One Plugin, one prelude.
- Simulations never call one another; coupling happens through shared voxels and events.
- Edits are gathered per frame and applied in a defined order, so results do not depend on the system order within a frame.
- Each simulation can be disabled without breaking the others.
