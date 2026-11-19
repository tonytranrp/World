# World_Cpu_Ref Source

## Purpose

This folder is the module tree of the reference renderer crate. It defines the crate's public surface and arranges the code into three concerns that mirror the stages of the GPU's primary pass: producing rays, tracing them, and recording and comparing results.

## Scope

- The crate entry point and its small public API: render a scene from a camera into a reference G-buffer, and compare buffers.
- Shared conventions for the whole crate: axis directions, pixel-center rules, depth and normal encodings, and material-id conventions, defined once so the GPU pass can copy them.
- One subfolder per concern: camera (ray generation), trace (traversal and lighting queries) and output (buffers, images and comparison).
- Glue that runs the pipeline for a frame, and a deterministic way to split pixel work across threads when a scene is large enough to want it.

## Out of scope

- Voxel storage, edits and production ray queries: World_Core.
- Terrain or prop generation: World_Gen.
- GPU code, shaders and Bevy integration: World_Render and assets/shaders.
- Test scenes and assertions that exercise the crate: the tests folder.

## Relationships

The modules form a simple pipeline: camera produces rays, trace consumes them against a World_Core tree, output collects what trace returns. Nothing flows backward. The crate depends on World_Core and is consumed by World_Render's test suites and by tools that dump reference images.

## Rules

- No Bevy, wgpu or windowing dependency.
- Parallelism, if used, never changes results: each pixel is computed independently and written to a fixed location.
- Modules are small and readable; the source is meant to be audited line by line against the GPU algorithm.
- Floating-point operations are written in a fixed order that the GPU code can follow, and fused operations are avoided unless the GPU also uses them.
- Public items document the convention they implement, not only their type.
