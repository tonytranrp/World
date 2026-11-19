# Mesh_To_Voxel Source

## Purpose

The source tree of the Mesh_To_Voxel converter. It holds the command-line entry point and the conversion pipeline: load a mesh, voxelize it, transfer color to materials, and write baked output. The pipeline logic sits in a layer separate from the entry point so that it can be tested without a command line.

## Scope

- The entry point that parses arguments and runs one conversion or a batch.
- Loading stage: decoding mesh files into triangles, normals, UVs, materials and textures.
- Voxelization stage: triangle-to-grid rasterization (conservative overlap tests against voxel cells), interior fill for closed meshes, and thin-surface handling.
- Color and material stage: sampling textures or vertex colors, quantizing to a palette, mapping to engine material ids.
- Writing stage: building grids and emitting World_Core's baked format, with size and pivot data.
- Diagnostics: reports of repaired or dropped geometry.

## Out of scope

- The baked format and material table, which belong in World_Core.
- Importing MagicaVoxel files, which belongs in Bake_Vox.
- Runtime asset loading, which lives in the engine crates.

## Relationships

Depends on World_Core. Invoked by developers or Xtask. The stages are internal; no other crate depends on them.

## Rules

- Stages are pure given their inputs and parameters, so results are reproducible.
- Triangle coverage tests are conservative and exact enough that a surface never leaves gaps at the chosen voxel size.
- No hidden global state or reliance on the working directory.
- Errors are structured and mapped to clear messages and exit codes at the entry point.
