# 0008: Deterministic floating point in terrain generation

Status: accepted, 2026-11-19

## Context

Decision 0006 stores edits as differences from regenerated terrain. If the
generator gives a different voxel on another machine, a saved edit lands on the
wrong surface. The plan already requires that the same seed and chunk give the
same voxels on Windows and macOS (Intel and Apple Silicon). Fast SIMD noise
libraries choose instruction sets at runtime and use fused multiply-add on some
paths, and platform math libraries differ in their last bits. The research is in
`../research/cpu-voxel-world-tiers.md`.

## Decision

- Terrain generation uses plain `f32` arithmetic: no fused multiply-add (no
  `mul_add`, no libraries that emit it), no platform math library functions
  (`sin`, `exp`, `powf` and the like), and no runtime selection of instruction
  sets. Where a transcendental is needed, it is a polynomial written in our code.
- Noise comes from a scalar library that meets those rules or from our own port,
  checked against its reference.
- Every generator change is versioned. A save records the generator version it
  was made with.
- CI hashes generated chunks for fixed seeds and compares them with golden hashes
  on Windows x86_64, macOS x86_64 and macOS arm64.

## Consequences

- Generation is slower than with the fastest SIMD libraries: a scalar library
  produces about 70 million 2D samples per second per core. A bottom-up builder,
  coarse-lattice 3D noise and skipping of empty or solid subtrees keep chunk
  generation in the tens of milliseconds.
- Fixed-point noise remains the fallback if the golden hashes ever disagree
  across platforms.
- Old generator versions must stay in the code for as long as saves made with
  them are supported.
