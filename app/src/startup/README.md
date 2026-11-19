# Startup

## Purpose

This folder owns boot ordering: the exact sequence that takes a resolved configuration to a running Bevy application. Order matters because the GPU device is created once at startup, the capability tier depends on the device, and render plugins must be registered to match the tier.

## Scope

- Sequencing: initialize logging, resolve configuration, select the wgpu backend set, create the device, read adapter features and limits, choose the tier, configure task pools, register plugins, run.
- Capability tier selection: tier A (ray queries, mesh shaders), tier B (compute and storage buffers) or tier C (unsupported, with a clear error). Log the chosen backend and tier.
- Applying `--mac-floor`: force tier B, set a 720p internal resolution, cap the brick pool, and limit worker threads to four.
- Plugin assembly: Bevy defaults, the Engine plugins appropriate for the tier, the game plugin, and development-only additions such as the asset file watcher and inspector.
- Preferring the high-performance adapter so dual-GPU machines use the discrete GPU.
- Platform setup that must happen before the window opens.

## Out of scope

- Argument parsing belongs in `cli`, and reading or writing the settings file in `settings`.
- Any behavior that happens once the app is running belongs in the plugins it registers.
- Tier-specific rendering code belongs in World_Render; startup only decides which plugins to register.

## Relationships

Consumes the resolved configuration from `cli` and `settings`. Depends on the Engine crates' plugins and the game plugin. Nothing else depends on it.

## Rules

- Order is explicit and each step has one responsibility.
- The device and tier are decided once; changing them requires a restart.
- Failures at this stage produce a readable message and a nonzero exit, never a panic backtrace as the user's first impression.
- Mac-floor mode must change only limits, never code paths that differ from a real tier B device.
