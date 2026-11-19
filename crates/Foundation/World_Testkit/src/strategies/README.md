# Strategies

## Purpose

This module generates random test inputs. It provides proptest strategies for voxel positions, materials, sequences of edits and rays, so every crate's property tests draw from the same well-shaped distributions and failures shrink to small, readable cases.

## Scope

- Positions and boxes within given bounds, including the boundary cells where off-by-one errors live.
- Material ids weighted toward a small set, so edits overlap, merge and overwrite each other often, with the empty material included.
- Edit sequences of single voxels, box fills and carves, with length taken from the profile's edits-per-case figure.
- Ray generators: rays from outside the volume, from inside solids, along axes, grazing voxel edges and corners, and rays that miss or leave the world.
- Shrinking that prefers shorter sequences, smaller boxes, positions nearer the origin and simpler materials.

## Out of scope

- Applying edits or casting rays: dense and the code under test.
- Deciding how many cases run: profile.
- Strategies specific to one crate, such as worldgen seeds or physics scenes. Those live with that crate.
- Fixture files.

## Relationships

Builds on dense for edit and ray types and on profile for sequence length. Used by the property tests of World_Core, World_Cpu_Ref and this crate.

## Rules

- Every strategy takes its bounds as arguments. None assumes a world size.
- Output is fully determined by the proptest seed. No wall clock, no global random state.
- Awkward cases are generated on purpose, never left to chance.
- Failing cases are kept as proptest regression inputs and committed with the fix.
