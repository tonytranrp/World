# Bake_Vox

## Purpose

Bake_Vox (package `bake_vox`) is an offline command-line tool that converts MagicaVoxel `.vox` files into the engine's baked voxel format. Artists and developers author props, tools and character parts in MagicaVoxel; this tool turns them into data the game loads directly with no parsing of the authoring format at runtime.

## Scope

- Reading `.vox` content: models, palettes, scene graph transforms, and multiple models per file.
- Mapping the file's color palette and per-voxel palette indices onto the engine's material ids through a mapping table, including emissive and other material flags where the source provides them.
- Producing the baked format defined by World_Core: dense grids for dynamic objects and rig parts, with per-object voxel size (1/64 m by default, 1/128 m for held items) recorded in the output.
- Splitting a scene into named parts so each can become a rigid part of a rig, and recording pivots and attachment points.
- Validation: reporting unmapped colors, oversized models and disconnected pieces.
- A command-line interface for single files and for batch folders.

## Out of scope

- The baked format definition, which belongs in World_Core.
- Triangle mesh conversion, which belongs in Mesh_To_Voxel.
- Loading baked data in the game, which belongs in the engine crates that consume it.
- Procedural content, which belongs in World_Gen.

## Relationships

Depends on World_Core for the format and material mapping types. Called by developers and by Xtask for batch asset builds. Nothing depends on it.

## Rules

- Output is deterministic and byte-stable for identical input and mapping.
- Source files are never modified; baked outputs are written elsewhere.
- Errors name the offending file, model and voxel position.
- The tool never ships in the game.
