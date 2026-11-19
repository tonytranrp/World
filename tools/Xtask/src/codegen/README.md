# Codegen Command

## Purpose

This folder implements the command that generates shader-side code from Rust-side definitions. The authoritative values (voxel size, brick and node dimensions, child masks, material table layout, palette limits, buffer strides) live in World_Core. Shaders must agree with them exactly, so the command writes them into WESL modules instead of letting anyone retype them.

## Scope

- Collecting the constants and layout descriptions from World_Core that shaders depend on.
- Emitting WESL modules with named constants and struct layouts that mirror the Rust types.
- A check mode that regenerates in memory and fails if the checked-in output differs, so drift is caught in CI.
- Stable formatting, ordering and header comments marking the output as generated.
- Layout assertions: verifying sizes and alignments of GPU-visible structures on the Rust side match the emitted definitions.

## Out of scope

- Writing the shaders themselves, which belongs to World_Render.
- Defining the constants, which belongs to World_Core.
- Shader compilation or hot reload, which are handled by Bevy and the asset watcher.
- Generating anything for the runtime game binary other than shader constants.

## Relationships

Called through the Xtask dispatcher. Reads World_Core. Its output is consumed by the shader modules owned by World_Render. The dependency runs from World_Core into shaders, never the reverse.

## Rules

- World_Core is the single source of truth; generated files are never edited by hand.
- Output is byte-stable for unchanged input.
- Generation fails loudly if a mirrored constant cannot be expressed in the shader language.
- The command touches only generated outputs and nothing else.
