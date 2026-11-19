# Assets

## Purpose

This folder holds everything the game loads at runtime through Bevy's asset server rather than compiles into the binary: shaders, tuning data, rig definitions, authored voxel content, audio and UI resources. It is the content side of the project. The Rust crates define behavior and data types; this folder supplies the values and resources those types consume.

The folder is watched by Bevy's `file_watcher` feature in development, so a saved change to a shader, a config or a rig reaches the running game without a rebuild. That hot-reload loop is the main reason content lives here and not in source code: anything a developer wants to tune by editing and saving belongs in this tree.

## Scope

- `shaders/`: WESL shader modules for the compute voxel renderer (shared common code, traversal, and one folder per render pass).
- `config/`: RON data for materials, worldgen parameters, physics constants and render tier settings, tuned live through bevy-inspector-egui.
- `rigs/`: RON rig definitions (bone names, lengths, joint limits), starting with the player rig.
- `models/`: authored voxel content, both MagicaVoxel `.vox` sources and baked output, split into player, tools and props.
- `audio/`: sound assets, including material-keyed break and impact sounds and ambience.
- `ui/`: fonts, icons and UI images. Triangle-based rendering is allowed for UI only.

## Out of scope

- Rust code, including the Rust side of render passes. That lives in the crates (for example World_Render for pass setup and bind groups).
- Test fixtures, golden images and replays. Those live in the workspace-root `tests/` folder.
- Packaging metadata such as bundle manifests. That lives in `packaging/`.
- Triangle meshes for the world, props or characters. World content is voxel data only.
- Generated or temporary build output.

## Relationships

- Loaded by `World_Render` (shaders, tier settings), `World_Game` (materials, tuning), `World_Physics` (physics constants), `World_Anim` (rigs) and `World_Gen` consumers (worldgen parameters), all through the asset server in `app/`.
- Produced in part by `tools/Bake_Vox` and `tools/Mesh_To_Voxel`, which write baked voxel files into `models/`.
- `tools/Xtask` generates shader constants from `World_Core`, so shader code and Rust stay in agreement.
- `packaging/` copies this tree into each platform bundle, so everything here must be relocatable.

## Rules

- Paths are relative to this folder; no absolute paths or machine-specific values.
- Large binary content (`.vox`, images, audio, baked voxel files) is stored through Git LFS.
- Data files are plain text (RON, WESL) so they diff and hot-reload cleanly.
- Content never encodes assumptions the code does not validate: a malformed config must fail with a clear error, not a crash.
- Names are lowercase with underscores, matching the module or pass they serve.
