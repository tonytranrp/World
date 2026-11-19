# sand

## Purpose

This folder simulates granular and falling materials with a cellular automaton: sand, gravel, snow, loose dirt, and anything the material table marks as granular. Voxels of these materials fall, slide off slopes, and settle into piles.

## Scope

- The automaton: per-voxel rules applied over the active region. A granular voxel falls into empty space below, otherwise slides to a free diagonal neighbor according to the material's angle of repose, otherwise rests.
- Update order and fairness: scan order alternating or randomized per step with a deterministic generator, so piles do not skew in one direction.
- Active-region tracking: only voxels that moved, or neighbors of changed voxels, are visited. Settled regions go to sleep and wake on nearby edits.
- Movement batching: gathering moves into a batch applied through World_Core's edit API, with conflicts (two voxels into one cell) resolved by a fixed rule.
- Interaction with other voxels: granular material displacing, burying, or being supported by other materials, and spilling over edges when support is destroyed.
- Handing off to rigid bodies: large cohesive masses are not simulated here, they belong to physics.

## Out of scope

- Fluids: fluids.
- Rigid-body debris and islands: World_Physics.
- Rendering sand: World_Render sees only voxel edits.
- Material properties: World_Core.

## Relationships

Depends on World_Core (tree access, edit API, granular flags in materials) and budget. Depended on by the crate's schedule. Its edits trigger World_Physics collider rebuilds like any other edit. Independent of stress, fire, and fluids.

## Rules

- Mass is conserved: a move relocates a voxel and never creates or destroys one.
- Rules are local and per-voxel; no global solve.
- Work is bounded by the shared budget and resumable between frames.
- Results are deterministic for a fixed seed and edit history, and match a dense-grid reference automaton.
