# World_Core

## Purpose

World_Core is the heart of the game: the authoritative CPU representation of the voxel world. It defines what a voxel is, how voxels are addressed, how they are stored in a sparse 64-tree (4x4x4 children per node, a 64-bit child mask per node), how edits are expressed and applied, and how the result is saved and loaded. The GPU only ever mirrors what lives here. The crate has no Bevy dependency and is the first thing tests exercise.

## Scope

- The shared constants in one module: voxel size of 1/64 m, collision scale of 1/16 m (a coarse cell spans several voxels), brick dimension, tree depth and fan-out. Xtask mirrors these into WESL so shaders cannot drift.
- Coordinate types and conversions between metres, voxels, collision cells, bricks, chunks and tree paths.
- The sparse 64-tree with palette-compressed materials, mip levels for distant rays, node storage and the mutation path.
- Materials: u16 material ids, per-node palettes, and the material table of color, roughness, emission, density, strength and friction.
- Edit operations, batches, dirty tracking and the overlay of player and destruction edits over a regenerated base.
- Binary serialization of trees and overlays.
- CPU ray traversal over the tree and the brute-force DDA oracle used to check it.

## Out of scope

- Procedural generation of terrain and props: World_Gen.
- Reference image rendering and shading: World_Cpu_Ref.
- GPU buffer layouts, brick pool streaming and shaders: World_Render and assets/shaders.
- Rigid bodies, colliders, island splitting, stress, fire, sand and fluids: World_Physics and World_Sim.
- Dense per-object grids as render objects are owned here only as data; their physics and drawing live in the Engine crates.

## Relationships

World_Core depends on no other workspace crate. World_Gen, World_Cpu_Ref and every Engine crate depend on it. Xtask reads its constants. Bake_Vox and Mesh_To_Voxel produce data in its formats.

## Rules

- No Bevy, wgpu, physics or windowing dependency.
- Integer math for all addressing; floats only at the metre boundary.
- Trees are canonical: equal content yields equal structure, so hashes and tests compare reliably.
- Edits always report the set of touched bricks, which drives GPU upload and collider rebuilds.
- Saved bytes are deterministic, little-endian and versioned.
- Every public structure is tested against a dense reference array.
