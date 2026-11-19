# Tier

## Purpose

Decides what the current machine can render and exposes that decision to the rest of the crate. The renderer runs on whatever backend wgpu selects (DX12, Vulkan, Metal), but the feature set differs per adapter, so capability tiers, not backend plumbing, are how the engine adapts.

## Scope

- Reading wgpu adapter features and limits once at startup (compute support, storage buffer limits, workgroup sizes, ray queries, mesh shaders, buffer size caps) and mapping them to a tier: A for ray-tracing GPUs with extras such as Solari and DLSS, B for baseline compute with software GI, C for unsupported adapters.
- The tier resource and capability flags other modules query, for example whether hardware ray queries or DLSS-RR may be used.
- The `--mac-floor` profile: forces tier B, 720p internal resolution, a capped brick pool and reduced worker threads, so the 2019 MacBook Pro budget can be tested on a development PC.
- The tier C path: a clear, user-readable error instead of a half-working renderer.
- Derived budgets that depend on tier and adapter limits, such as the maximum brick pool size and internal resolution defaults, before user settings override them.

## Out of scope

- Backend selection and the `--backend` flag: the `app` binary, which creates the GPU device.
- Building pipelines or choosing shader variants: `passes` and `brick_cache` read the tier and decide for themselves.
- Runtime performance scaling (dynamic resolution): the upscale pass.

## Relationships

Depends only on Bevy and wgpu. Everything else in the crate may read it, and `graph` consults it to decide which nodes to register. The `app` binary feeds it command-line profile choices.

## Rules

- Detection runs once, before any render resource is created; the GPU device is fixed for the life of the process, and a backend change applies on restart.
- Tier B is the baseline and must be fully functional. Tier A only adds; it never changes what tier B produces.
- Capability is inferred from adapter data, never from vendor or OS names.
- Tier C halts with an explicit message and never falls back silently.
- `--mac-floor` changes only budgets and tier, not code paths, so a PC run exercises the same code as the Mac.
