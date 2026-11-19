# Prop Models

## Purpose

This folder holds authored voxel props: objects placed in the world or scattered by worldgen that are authored by hand or converted from meshes, rather than built from SDF graphs. Examples of the kind of content are furniture, crates, fixtures, set dressing and landmark structures.

## Scope

- `.vox` sources and baked voxel grids for props.
- Metadata for each prop: voxel scale, pivot, bounding box and any anchor points (where it attaches to ground or walls).
- Material assignment per prop through the shared material table, so the break and impact behavior of a crate is defined by its materials and not by special cases.
- Props that become dynamic objects when freed or destroyed. Their grids are small and dense, suited to box-march rendering and a voxel collider.

## Out of scope

- Procedural props such as rocks, trees and ruins. Those are SDF graphs and grammars in `World_Gen`, voxelized at whatever resolution the LOD needs, with parameters in `assets/config`.
- Player body parts (`../player`) and held tools (`../tools`).
- Terrain chunks or anything regenerated from a seed.
- Triangle meshes as runtime content.

## Relationships

- Sources feed `tools/Bake_Vox` and `tools/Mesh_To_Voxel`; baked output is loaded by `World_Game` and placed by worldgen rules.
- Materials resolve against `assets/config`.
- Physics derives mass and inertia from per-voxel material density, so palette choices affect behavior; `World_Physics` builds colliders from the grid.
- Packaged by `packaging/`.

## Rules

- All voxel and image content is stored through Git LFS.
- Props must be self-contained: no dependency on other props or on a specific world location.
- Keep grids as small as the detail allows; very large props belong in the world tree, not as dynamic objects.
- Baked files are reproducible from their sources and are never edited by hand.
