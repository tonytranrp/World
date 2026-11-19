# World_Core Source

## Purpose

This folder is the module tree of the World_Core crate. It defines the crate's public surface and arranges the implementation into deep, single-responsibility module folders. Deep modules keep each concept's code together, which makes hot patching and targeted testing practical.

## Scope

- The crate entry point, which re-exports a deliberately small public API.
- The single home of the shared constants (voxel size, collision scale, brick dimension, tree fan-out and depth). Code anywhere else in the workspace reads them from here.
- Basic shared types that every module needs and that are too small to justify their own folder, such as common error and result types.
- One subfolder per concept: coordinates, edits, materials, ray traversal, serialization, and the tree with its own nested modules.

## Out of scope

- Concepts that need a runtime, a window, a GPU or an ECS. Those belong to the Engine crates.
- Generation logic: World_Gen.
- Test code and benchmarks: the sibling tests and benches folders at the crate root.
- Large grab-bag utility modules. If something is reusable, name the concept and give it a folder.

## Relationships

Module dependencies flow in one direction: coordinates and materials are leaves; the tree builds on them; edits and ray traversal build on the tree; serialization builds on the tree and edits. No module may import from a module that depends on it. World_Gen, World_Cpu_Ref and the Engine crates consume only the public API re-exported here, never reach into module internals.

## Rules

- Everything is plain Rust: no Bevy types, no wgpu types, no global state.
- Constants appear exactly once. Anything the GPU must agree on is defined here so Xtask can generate the WESL mirror.
- Public items are documented with the invariant they uphold, not only their signature.
- Unsafe code is confined to the smallest possible modules, documented, and covered by property tests.
- Module folders are snake_case and own their private helpers; cross-module sharing goes through the public surface of the owning module.
