# Dense

## Purpose

This module is the oracle. It holds a plain, fixed-size 3D array of material ids that supports edits and ray casts in the most obvious way possible. The sparse 64-tree in World_Core must agree with it after any sequence of edits. When the two disagree, the tree is wrong.

## Scope

- The dense model: a fixed-size three-dimensional array of material ids with bounds-checked reads and writes.
- Edit application over the array: single voxels, box fills and carves, with the semantics the real edit API promises.
- A ray cast by voxel DDA over the array, returning the first solid voxel with its material, entered face and distance. Its own correctness is established by tests against a brute-force walk.
- A trait describing the minimal voxel-store behavior (read a voxel, apply an edit), which the tree under test is adapted to in the tests of the crate that owns it.
- A divergence checker that compares a store with the model and, on a mismatch, reports the first differing voxel, both materials and the edit that preceded it.

## Out of scope

- Generating edits and rays: strategies.
- Comparing the results of two tracers: hits.
- Anything sparse, compressed or fast. Speed is not a goal here.
- Hashing the model or recording it: hash and replay.

## Relationships

Built on plain integers and material ids, with World_Core supplying shared constants where needed. Used by strategies for bounds and edit types, by hits as the stand-in CPU side, and by the tests and benches of this crate and of World_Core.

## Rules

- Written to be read: no acceleration structure, no unsafe code, no cleverness.
- Nothing from the production tree's implementation is reused.
- Behavior at and beyond the bounds is defined and documented, including rays that start outside or leave the array.
- The same model and the same edits give the same result on every machine.
