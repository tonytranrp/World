# import

## Purpose

This folder brings authored voxel art into the rig system. Its main job is reading MagicaVoxel .vox files and turning their models into rig parts: dense voxel grids with palette colors mapped to World_Core materials.

## Scope

- Parsing .vox content: model chunks, palettes, and scene graph transforms, including multi-model files used to hold a character's parts.
- Mapping .vox palette indices to material ids in the World_Core material table, with a mapping config so authors choose which palette entries are metal, cloth, skin, and so on.
- Converting model coordinates and axis conventions into the project's coordinate system and 1/64 m voxel size, with an optional resolution for fine parts such as hands and tools.
- Assigning imported models to rig bones by name, using naming conventions in the source file or a mapping asset.
- Bevy asset-loader integration so imported parts hot-reload when the source file changes.
- Validation: reporting empty models, oversize grids, unmapped palette entries, and missing bones.

## Out of scope

- Offline bulk baking and mesh-to-voxel conversion: the tools folder.
- An in-game part editor: not part of this crate's import path.
- The part model and attachment: parts.
- Skeleton data: rig.

## Relationships

Depends on World_Core (grids, materials) and rig (bone names), and on the third-party .vox parsing library. Depended on by parts. Anything that needs voxel art from outside the engine enters through here.

## Rules

- Import produces data only; it spawns nothing and edits nothing in the world.
- Material mapping is explicit; unmapped colors are reported, never silently guessed.
- Import is deterministic: the same file yields the same grid.
- Format-specific types never leak past this module.
