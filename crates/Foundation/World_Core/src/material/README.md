# Materials

## Purpose

This module defines what a voxel is made of. Each voxel stores a u16 material id, resolved through a per-node palette, and a global material table maps ids to physical and visual properties. The same table drives rendering, friction, density, strength and break behavior, so materials are described once.

## Scope

- The material id type and its reserved values, such as empty space.
- The material definition record: base color, roughness, emission, density, strength, friction, and any other per-material scalar the engine needs.
- The material table: dense lookup by id, validation of definitions, and derived quantities such as mass per voxel from density and voxel volume.
- Palette types used by tree nodes: a small list of ids per node with compact indices, plus the repacking rules when a palette grows or shrinks.
- Serde-friendly data types so that tables can be filled from RON configs by higher layers.

## Out of scope

- Reading files, hot reload and asset pipelines: Engine crates load configs and hand parsed tables to this module.
- GPU-packed material buffers and texture atlases: World_Render.
- Sounds, particles and gameplay reactions tied to a material: World_Physics, World_Sim and World_Game.
- Generator-side rules about which material appears where: World_Gen surface and props modules.

## Relationships

A leaf module with no dependencies beyond the shared constants. The tree node module uses palette types; edits and serialization use ids and the table; World_Gen emits ids; World_Physics reads density, strength and friction; World_Render reads color, roughness and emission.

## Rules

- Material ids are stable identifiers; reordering or renumbering is a data migration, never a silent change.
- Normals are never stored; they are derived from neighboring voxels at render time.
- Palette indexing is lossless: decoding any node always returns exactly the ids that were written.
- The table is immutable while shared; changes are applied by replacing the whole table.
- All numeric fields have documented units and valid ranges, and validation rejects out-of-range values.
