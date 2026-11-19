# Passes

## Purpose

Category folder for the render passes: each subfolder is one stage of building a frame, from primary ray traversal through lighting, reflections and upscale to final composition. A pass folder holds the Rust side of that stage (pipelines, bind groups, dispatch or draw, per-view resources); the WESL shader source lives in `assets/shaders/passes/<same name>` and is hot-reloaded.

## Scope

- One subfolder per pass or pass family, named identically to its shader folder: `primary`, `dynamic_box`, `lighting` (with `shadow`, `ao`, `gi`), `reflections`, `upscale`, `composite`.
- The frame order these express: primary rays, dynamic-object box march into the same G-buffer, lighting v1 (sun shadow rays and AO), lighting v2 (radiance cache GI), optional lighting v3 on tier A (Solari, DLSS-RR), reflections, temporal accumulation and upscale, composition with UI and post effects.
- Conventions shared by every pass: how a pass declares its pipelines, how it reads the G-buffer and the brick pool, and how it exposes a node to `graph`.

## Out of scope

- Shared infrastructure: the G-buffer in `gbuffer`, the pool and streaming in `brick_cache`, ordering in `graph`, capability decisions in `tier`.
- Shader source: `assets/shaders`. Shared traversal code lives in `shaders/traversal` and `shaders/common` and is imported by passes, never copied.
- Tunable numbers: `assets/config`.
- Anything outside rendering.

## Relationships

Depends on `gbuffer`, `brick_cache` (for the pool all tracing passes walk), `tier` and, through `graph`, on Bevy's Core3d graph. `graph` depends on the nodes passes expose. Passes depend on each other only through the G-buffer and documented history textures, never by calling one another.

## Rules

- A pass owns its own intermediate textures and pipeline state.
- Any pass that traces rays uses the shared traversal WESL modules, so behavior stays identical and testable against the CPU reference.
- Tier-A-only passes are separate and gated; tier B must produce a complete image without them.
- Adding a pass means adding a folder here and a matching shader folder, not extending an existing pass with unrelated work.
- Passes never read the CPU tree; they see only the GPU mirror.
