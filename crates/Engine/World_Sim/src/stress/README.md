# stress

## Purpose

This folder simulates structural integrity: whether built or natural structures hold up under their own weight, and what happens when they do not. It extends the binary question of whether a piece is connected to bedrock into a graded one, so overhangs, bridges, and thin supports can fail.

## Scope

- The support graph: a coarse graph built over voxels or groups of voxels, with edges carrying capacity derived from material strength and cross-section, and nodes carrying weight from material density.
- Load propagation: pushing weight from each node down through supports to anchors, computing stress per edge, with incremental updates when only a local region changed.
- Failure rules: an edge or node whose load exceeds capacity fails, which is reported as damage, and the collapse is then handled by island detection and falling in World_Physics.
- Progressive collapse: re-evaluating neighbors after a failure, spread across frames.
- Region handling: stress is computed only for regions that changed or are near activity, and settled structures cost nothing.
- Warning data, such as stress levels that gameplay or rendering can use for creaking, cracking, and visual stress cues.

## Out of scope

- Island detection and anchor classification: World_Physics/destruction, which this module builds on.
- Removing voxels and spawning debris: damage and debris in World_Physics.
- Fire weakening materials: fire, which acts by changing voxels or material state that stress then reads.
- The material table: World_Core.

## Relationships

Depends on World_Physics (island and anchor results, damage events), World_Core (tree reads, materials), and budget. Depended on by the crate's schedule. Independent of the sand, fire, and fluids modules.

## Rules

- Stress is computed on the coarse graph, never per render voxel.
- Capacity and weight come only from the material table.
- Work is time-sliced under the shared budget.
- Failure produces damage events, never direct tree writes.
- Results match a brute-force load computation on small structures.
