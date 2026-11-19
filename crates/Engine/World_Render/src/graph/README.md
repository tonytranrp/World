# Graph

## Purpose

Integrates the voxel renderer into Bevy's render pipeline. This folder defines how World_Render's passes become nodes in the Core3d render graph, what order they run in, and how main-world data (camera, edits, dynamic objects) is extracted into the render world each frame.

## Scope

- Render-graph node labels and the edges between them, expressing the frame order: brick streaming, primary traversal, dynamic-object box pass, lighting, reflections, temporal accumulation and upscale, composition, then Bevy's UI and post effects.
- Registration of nodes according to tier, so tier A extras only appear on capable adapters.
- Extraction systems that copy the minimum needed state into the render world: camera matrices and jitter, tree edit deltas, dynamic-object boxes, sun parameters and settings.
- Render-world schedule sets and queue/prepare/cleanup ordering shared across passes.
- Frame-level synchronization between the feedback readback of one frame and the streaming of the next.

## Out of scope

- The work each node does: pass folders under `passes` and `brick_cache`.
- Pipeline and shader creation: the pass that owns the shader.
- Render-target definitions: `gbuffer`.
- Backend and tier choice: `tier` and the `app` binary.

## Relationships

Depends on Bevy's render crates, `tier` for gating, and the public node interfaces of `passes`, `brick_cache` and `gbuffer`. The plugin in the crate root calls into it. Nothing else depends on its internals.

## Rules

- All custom work runs as nodes in the Core3d graph; no parallel render path bypasses it.
- Order is declared with explicit edges and system sets, never by registration order.
- Extraction copies deltas, not whole structures, and never reads the CPU tree directly from the render world.
- Voxel passes must complete and write depth before Bevy's rasterized transparency, gizmos and UI run.
- Graph code contains no pass logic; it only wires.
- Node availability is decided by tier at startup and does not change at runtime.
