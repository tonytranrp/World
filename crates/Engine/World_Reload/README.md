# World_Reload

## Purpose

World_Reload lets the running game change without a restart. It is where data reloading and code reloading meet. Tunable data lives in RON files, and this crate turns each file into a typed, validated value that follows every save. Rust code can be hot-patched while the game runs, and this crate makes sure that setup code, which ran only once at startup, runs again with the new code. In both cases a bad change is caught and reported, and the last good state stays live.

## Scope

- Typed config assets: one loader per config type, parse then validate, apply only on success, keep the last good value on failure.
- The live value of each config as a resource that systems read, with change detection on every reload.
- A registry of every config with its asset path, where its live value came from, how many changes were applied or rejected, and the last error.
- Type-erased access by config name, so tools can read, apply and save configs without knowing their types; saving is atomic.
- The bridge from Bevy's hot-patch notification to the engine's own code-reload message.
- Reloadable setup: systems that run at startup and again after each code reload, with the entities they spawned removed first.

## Out of scope

- The config types themselves and their validation rules: the crates that own them (materials in World_Core, physics tuning in World_Physics, and so on).
- The files: `assets/config`.
- The developer panels that display and edit configs: World_Game's ui domain.
- Watching files, running dx and building patches: Bevy's asset server, the dx CLI and Xtask.
- Player settings stored outside the asset tree: the app's settings module.

## Relationships

An Engine-layer crate depending on Bevy and on World_Core (for the config trait). Every other Engine crate, World_Game and the app may depend on it; it depends on none of them. Tests use World_Testkit_Bevy's asset sources.

## Rules

- Nothing is applied before it parses and validates. A rejected change changes nothing but the status.
- Readers never wait: every config has safe defaults from the start.
- Registering the same config twice from the same path is harmless; from two paths it is a bug.
- Setup that should follow code edits is registered as reloadable and tags what it spawns, so a reload never duplicates entities.
- The crate stays small and stable: it is infrastructure, not gameplay, and rarely changes.
