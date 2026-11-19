# Serialization

## Purpose

This module turns in-memory world data into bytes and back. Because the base world is regenerated from the seed, saves hold only the player's edits and destruction, so the central job here is encoding the edit overlay compactly and reading it back exactly. Trees can also be serialized for baked assets and for dynamic object grids.

## Scope

- The binary encoding of tree subtrees, bricks, palettes and per-chunk edit overlays.
- A versioned container with a header, a format version, per-chunk records addressable by chunk coordinate, and an integrity checksum.
- Migration rules between format versions, expressed as pure functions over decoded data.
- Compression of runs, uniform regions and palettes where it pays off, kept deterministic.
- A canonical content hash of a chunk or subtree, used by golden tests to detect nondeterminism.
- Streaming-friendly reading so one chunk's records can be loaded without decoding the whole save.

## Out of scope

- File system access, save slots, autosave scheduling and cloud storage: World_Game and the app.
- Text configs in RON (materials, worldgen parameters): handled by the layers that load assets.
- Network protocols: the game is single-player.
- Deciding what counts as an edit: the edit module.

## Relationships

It depends on coordinates, materials, the tree and the edit overlay types. World_Gen and World_Cpu_Ref use the content hash for golden tests; Bake_Vox writes baked voxel data through it; World_Game drives saves and loads.

## Rules

- Output bytes are fully deterministic: fixed endianness, fixed field order, no hash-map iteration order, no padding garbage.
- Round trip is lossless: decode(encode(x)) equals x, verified by property tests.
- Decoding untrusted or corrupt input returns an error and never panics or over-allocates.
- Every format change bumps the format version and ships a migration.
- Saving never stores data that the seed can regenerate.
