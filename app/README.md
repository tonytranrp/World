# App

## Purpose

The app folder is the game's executable: the only binary crate that players run. Its job is to turn a command line, a settings file and the machine's graphics capabilities into a configured Bevy application, then run it. It contains no game logic. Everything the player experiences comes from the plugins it assembles.

## Scope

- Backend selection: choosing DX12, Vulkan or Metal through a `--backend` flag, the `WGPU_BACKEND` environment variable or a settings entry, with automatic selection by wgpu as the default. One build per OS has every primary backend compiled in. A backend change applies on restart, because the GPU device is created at startup.
- The `--mac-floor` mode, which forces tier B, a 720p internal resolution, a capped brick pool and four worker threads, so the lowest-spec Mac can be approximated on a development PC.
- The settings file: loading, defaults, validation and saving of user preferences.
- Startup and boot ordering: argument parsing, settings load, creating the wgpu device, reading adapter features and limits to select a capability tier, configuring task pools, then registering plugins.
- Plugin assembly: adding Bevy's default plugins, the Engine crates' plugins and the game plugin, in an order that respects their dependencies, and enabling dev-only plugins such as the asset file watcher and inspector.

## Out of scope

- Gameplay, UI content and tools belong in World_Game.
- Rendering, physics, animation and simulation belong in the Engine crates.
- Voxel data and generation belong in Foundation crates.
- Packaging and release automation belong in `tools/` (Xtask).

## Relationships

Depends on every crate it assembles: the Foundation crates it needs, the Engine plugins and the game plugin. Nothing depends on it. It is the top of the order Foundation <- Engine <- Game <- app.

## Rules

- The binary stays thin; if code can be tested without a window, it belongs in a library crate.
- Tier detection happens once at startup and decides which render plugins register. Unsupported tiers (no compute shaders) end with a clear error, not a crash.
- Settings that need the device created first (backend, tier, pool size) take effect on restart, and this is stated to the user.
- The dev profile uses dynamic linking, opt-level 1 for workspace crates and 3 for dependencies.
- Hot patching and live asset reload must work from this binary without special code paths.
