# Tools

## Purpose

This folder holds the developer-facing programs that support the game but are not part of it: build automation, and offline converters that prepare content. Each tool is its own crate and workspace member. Tools run on a developer or CI machine, and none of them is ever shipped inside the game.

## Scope

- Xtask: the build and automation runner invoked through `cargo xtask`. It generates code, runs benchmarks against thresholds and packages releases.
- Bake_Vox: an offline converter from MagicaVoxel `.vox` files to the engine's baked voxel format.
- Mesh_To_Voxel: an offline converter from triangle meshes to voxel data.
- Any future offline utility with the same character: it consumes authoring formats, produces engine-ready data, and runs before the game starts.

## Out of scope

- Runtime code of any kind. The runtime never meshes, never parses authoring formats and never calls into a tool.
- Gameplay or engine features, which belong in `crates/`.
- Assets themselves; tools produce them and the assets folder stores them.
- The game executable, which lives in `app/`.

## Relationships

Tools may depend on Foundation crates to share formats and constants, for example World_Core for the baked voxel format and material table. They never depend on Engine or Game crates, and nothing in `crates/` or `app/` depends on a tool. The workspace includes each tool through the member pattern for this folder.

## Rules

- Tools are build-time or offline only and must not appear in the shipped game bundle.
- Output formats are defined by World_Core; tools write them and never invent private variants.
- Tools are deterministic: the same input produces byte-identical output, so baked assets and generated code diff cleanly.
- A tool exposes a command-line interface with clear exit codes so CI can use it.
- Keep dependencies out of the runtime build; heavy parsing or image libraries are acceptable here because they never ship.
