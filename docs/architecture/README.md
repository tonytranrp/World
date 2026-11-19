# Architecture

## Purpose

This folder explains how the system fits together. Where `../plans` argues for choices and `../decisions` freezes them, these pages describe the structure that results: which crate owns what, how data moves, and in what order things happen. They exist so a new contributor can build a correct mental model before reading code.

## Scope

- The crate graph: dependency directions between Foundation (World_Core, World_Gen, World_Cpu_Ref), Engine (World_Render, World_Physics, World_Anim, World_Sim), Game (World_Game), the app binary and the tools, and the rule that the core crates have no Bevy dependency.
- Data flow from the CPU 64-tree, the single source of truth, to the GPU: node and brick pools, the ray-guided feedback loop, LRU eviction, and how edits propagate.
- Frame build order: brick streaming, primary rays, dynamic objects, lighting, accumulation and upscale, UI and post.
- The physics loop: fixed 60 Hz step, voxel colliders on the coarser grid, destruction and island detection, debris particles, and how results return to the tree.
- Cross-cutting design: capability tiers, hot-reload layers, determinism and the save overlay on generated terrain.
- Diagrams that support these explanations.

## Out of scope

- The reasoning behind a choice and the alternatives. Those belong in `../decisions` and `../plans`.
- Notes about other projects and papers. Those belong in `../research`.
- API reference for types and functions, which comes from Rust doc comments.
- Descriptions of what each folder contains; each folder's own README covers that.
- Progress and history, which belong in `../devlog`.

## Relationships

- Pages are derived from the code layout in `crates/`, `tools/` and `assets/`, and must be corrected when the structure changes.
- They link to decision records for rationale and to research notes for background.
- They should agree with the plan; when they diverge, resolve the difference deliberately and record it.

## Rules

- Describe the system as it is intended to work, one subsystem per page.
- Name the owning crate for every behavior so responsibility is never ambiguous.
- Prefer diagrams plus short prose over long narrative.
- Keep the Bevy-free versus Bevy-dependent boundary visible in every diagram that crosses it.
