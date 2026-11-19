# Bake_Vox Source

## Purpose

The source tree of the Bake_Vox converter. It holds the command-line entry point and the conversion logic organized as a small pipeline: read a `.vox` file, map it to engine materials, build voxel grids and write the baked format. Keeping the logic in a testable library layer beneath a thin entry point lets the conversion be exercised without the command line.

## Scope

- The entry point that parses arguments and runs a conversion or batch.
- Reading stage: decoding the authoring format into an in-memory model list with transforms and palette.
- Mapping stage: translating palette entries to engine material ids through a configurable table.
- Build stage: producing grids sized and positioned per part, with voxel size, pivots and attachment data.
- Writing stage: emitting the baked format using World_Core's serialization.
- Diagnostics and reporting.

## Out of scope

- The definition of the baked format and material table, which belongs in World_Core.
- Mesh handling, which belongs in Mesh_To_Voxel.
- Runtime loading, which lives in the engine crates.

## Relationships

Depends on World_Core. Invoked by developers or Xtask. Its stages are internal and do not form a public API for other crates.

## Rules

- Stages are separable and pure where possible: given the same decoded input and mapping, output is identical.
- No global state, no network, no dependence on the working directory beyond explicit paths.
- Failures return structured errors, which the entry point converts to messages and exit codes.
- New input features add to a stage; they never bypass the mapping to engine materials.
