# World_Gen

## Purpose

World_Gen produces the base world. Given a seed and a chunk coordinate it returns the voxels of that chunk, and the same pair gives identical voxels on every machine, every platform and every thread count. The base world is never stored: saves keep only player edits and destruction as an overlay, and the base is regenerated on demand. That makes this crate the authority for what the untouched world looks like, and the reason a save moved between a Windows PC and a Mac still lines up.

## Scope

- The staged generation pipeline, from coarse to fine: continental scale (continents, heightmap, biomes, optional coarse erosion), regional scale (caves, rivers, cliffs expressed as signed distance fields), props (rocks, trees, ruins as SDF graphs and grammars), and surface detail (material layers chosen by depth and slope).
- Deterministic noise and hashing primitives that every stage shares.
- Chunk-addressed orchestration: deriving per-stage random streams from seed and coordinate, resolving dependencies between neighboring chunks without ordering effects, and writing results into World_Core structures.
- Resolution-aware voxelization, so props and terrain can be produced at the level of detail the requester needs.
- Parameter structs describing generation settings, designed to be filled from RON configs by higher layers.
- Parallel chunk generation on the CPU with a result that never depends on scheduling.

## Out of scope

- The edit overlay, applying edits, and saving: World_Core. Generation never reads edits.
- Streaming policy, deciding which chunks to generate and when: World_Render and World_Game.
- GPU generation. If a GPU path is ever added it must use integer math or stay visual-only, and it must not be authoritative.
- File loading and hot reload of RON assets: Engine crates.

## Relationships

World_Gen depends on World_Core only, and on nothing else in the workspace except development tooling. World_Render, World_Physics and World_Game consume its output through World_Core. World_Cpu_Ref may use it to build test scenes.

## Rules

- Determinism is the contract. Results depend on seed, chunk coordinate and parameters only.
- Only operations with identical results across machines are allowed in authoritative paths: integer math, basic IEEE arithmetic in a fixed order, and explicitly implemented noise instead of platform math libraries for transcendental functions.
- No wall clock, no global random state, no hash-map iteration order.
- Output must be seamless across chunk borders.
- A golden hash per seed and chunk guards against drift, and any intentional change to output is a world-version change.
