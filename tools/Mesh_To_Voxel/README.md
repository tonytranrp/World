# Mesh_To_Voxel

## Purpose

Mesh_To_Voxel (package `mesh_to_voxel`) is an offline command-line tool that converts triangle meshes into voxel data. It exists because the engine never meshes and never rasterizes world content at runtime: when art arrives as triangles (from a modeling tool or a downloaded model), it is voxelized here, once, before the game uses it.

## Scope

- Reading common mesh interchange formats with materials and textures.
- Voxelization at a chosen voxel size (1/64 m by default, 1/128 m for held items), using surface rasterization of triangles into the grid, optionally with interior filling by flood fill or winding-number tests for closed meshes.
- Color transfer: sampling textures or vertex colors at each surface voxel and quantizing to the engine's material and palette entries through a mapping.
- Handling of thin features, non-watertight meshes and degenerate triangles, with reporting of what was repaired or skipped.
- Optional splitting of a mesh into parts for use as rig parts.
- Writing the baked format defined by World_Core.

## Out of scope

- Runtime conversion of any kind. Nothing at runtime calls into this tool.
- The baked format definition, which belongs in World_Core.
- MagicaVoxel files, which belong in Bake_Vox.
- Procedural SDF voxelization, which belongs in World_Gen.

## Relationships

Depends on World_Core. Invoked by developers or by Xtask asset builds. Nothing depends on it.

## Rules

- Deterministic output: the same mesh and parameters yield byte-identical results.
- Voxel size and fill mode are explicit parameters, never inferred silently.
- Source meshes are never modified.
- The tool never ships in the game, and mesh libraries stay confined to it.
