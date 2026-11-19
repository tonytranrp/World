# World_Render

## Purpose

World_Render (package `world_render`) is the Bevy rendering crate for the voxel world. It turns the CPU 64-tree owned by World_Core into pixels without ever building triangle meshes: compute shaders trace rays through a GPU mirror of the tree, write a G-buffer, light it, accumulate it over time, upscale it, and hand the result to Bevy for UI and post effects. It also owns the decision of what the machine can run, through capability tiers.

## Scope

- The single public `Plugin` entry point and a prelude. Everything else is internal to the crate.
- Capability tier detection (A: ray-tracing GPUs with extras such as Solari and DLSS; B: baseline compute with software GI; C: unsupported, clear error) from wgpu adapter features and limits at startup, plus the `--mac-floor` profile that forces tier B, a 720p internal resolution and a capped brick pool.
- The ray-guided brick cache: a fixed GPU node pool and brick pool mirroring the CPU tree, a feedback buffer of missing-brick requests written by rays, CPU streaming under an MB-per-frame budget, LRU eviction, and coarse mips for far rays.
- Compute traversal of the sparse 64-tree into the G-buffer (depth, normal, material id, motion vectors), with depth also written into Bevy's depth texture.
- Dynamic objects drawn as oriented boxes whose fragment shader marches each object's dense grid into the same G-buffer.
- Lighting (sun shadow rays and AO, then a world-space radiance cache for GI, optionally Solari or DLSS-RR on tier A), reflections, temporal accumulation and upscale, and composition with Bevy's own passes.
- Custom render-graph nodes in the Core3d graph and the debug tooling for all of the above.

## Out of scope

- Voxel data, the 64-tree, edits and shared constants: World_Core. This crate reads them and never defines them.
- The CPU reference ray tracer used as ground truth: World_Cpu_Ref.
- Physics, destruction and island splitting: World_Physics. Animation and rigs: World_Anim. Simulation and gameplay: World_Sim and World_Game.
- Shader source. WESL files live in `assets/shaders`, mirroring the pass folders under `src/passes` (`shaders/common`, `shaders/traversal`, `shaders/passes/*`), and are hot-reloaded. Rust here owns pipelines, bind groups and dispatch, not the shader text.
- RON tuning data: `assets/config`.

## Relationships

Depends on World_Core (tree, constants, materials) and Bevy. It must not depend on World_Physics, World_Anim or World_Sim. World_Game and the `app` binary depend on it through the prelude, and tests compare it against World_Cpu_Ref. Shader constants are generated from World_Core by the Xtask codegen, so Rust and WESL cannot drift.

## Rules

- The CPU 64-tree is the single source of truth. The GPU holds a cache of it, never an independent copy of world state.
- GPU memory is fixed by pool configuration and scales with screen resolution, not world size.
- Tier B is the baseline. Tier A features are additive and gated by detected capability; code for them never changes tier B behavior.
- Traversal logic is written once as shared WESL modules and reused by every pass.
- Rust-side struct layouts used by shaders derive from World_Core constants, never from literals.
- No shader may require hardware ray tracing or mesh shaders outside tier-A-gated code.
