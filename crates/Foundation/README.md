# Foundation

## Purpose

Foundation is the lowest layer of the workspace. It holds the crates that define what the world is (voxel data, the sparse 64-tree, materials, edits, serialization), how the base world is produced from a seed, and how the world is rendered by a slow, exact CPU reference. Everything here is plain Rust with no Bevy dependency, so it compiles fast, is covered by property tests, survives engine upgrades unchanged, and could run on a server or in a command-line tool.

## Scope

Foundation contains exactly three crates, each in its own Pascal_Case folder with a snake_case package name:

- World_Core: voxel types, coordinate math, the sparse 64-tree, palette-compressed materials, edits and the edit overlay, serialization, and CPU ray traversal. It also owns the shared constants (voxel size of 1/64 m, collision scale of 1/16 m, brick dimension) in one place.
- World_Gen: the deterministic, CPU-authoritative procedural pipeline addressed by seed and chunk coordinate.
- World_Cpu_Ref: the slow, exact reference ray tracer that is ground truth for the GPU renderer.

Any new crate placed here must satisfy the same test: it is pure data and algorithms, usable without a window, a GPU, an ECS or a physics engine.

## Out of scope

- Anything that names Bevy, wgpu, a physics engine, a windowing system or an audio device. Rendering, physics bridges, animation and simulation live in the Engine crates (World_Render, World_Physics, World_Anim, World_Sim).
- Gameplay rules and content logic live in the Game layer (World_Game).
- Shader code lives in assets/shaders. Foundation only defines the constants and layouts that shaders must agree with.
- Binaries and baking utilities live in tools/ and app/.

## Relationships

Dependency direction across the repository is one way: Foundation <- Engine <- Game <- app. Foundation depends on nothing above it. Inside Foundation, World_Core depends on no workspace crate; World_Gen depends on World_Core only; World_Cpu_Ref depends on World_Core and may use World_Gen only as a development dependency for building test scenes. The Xtask tool reads the constants in World_Core and generates matching WESL definitions under assets/shaders, so Rust and shaders cannot drift. Bake_Vox and Mesh_To_Voxel in tools/ write voxel data through World_Core types.

## Rules

- Zero Bevy dependency, enforced for every crate in this folder and for their transitive dependencies.
- No global mutable state, no wall-clock time, no thread-count-dependent results.
- Determinism first: the same inputs produce bit-identical outputs on Windows and macOS, on Intel and Apple Silicon.
- Constants that the GPU also needs are defined once in World_Core and never repeated elsewhere.
- Each crate is validated against a simpler reference (dense array, brute-force DDA, analytic geometry) in its tests folder.
- Crates expose a small public surface and keep hot data structures deep inside module folders.
