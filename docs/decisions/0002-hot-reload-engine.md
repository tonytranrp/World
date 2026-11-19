# 0002: Hot-reload engine

Status: accepted, 2026-11-18

## Context

The plan's iteration-speed section lists five layers: asset hot reload,
data-driven tuning, Rust hot-patching, fast rebuilds and optional scripting.
Asset watching was already on. Nothing loaded config files, nothing could
patch code, and nothing re-ran setup after a patch. Research against Bevy 0.20.1
found:

- Bevy's `hotpatching` feature uses Dioxus `subsecond` 0.7.10. The `dx` CLI must
  be the same version. Every system in every schedule is patched; `Startup`
  systems are patched but never run again. Bevy writes a `HotPatched` message.
- On Windows, `bevy/dynamic_linking` and hot-patching cannot be combined
  (Dioxus #4693), and dx 0.7.10 cannot drive `rust-lld` (Dioxus #4694).
- The asset file watcher debounces for 300 ms. Shader imports are tracked as
  file dependencies, so editing an imported WESL module reloads its users.
- Several asset loaders can share the `.ron` extension: the asset server picks
  the loader by asset type when exactly one loader produces that type, and
  reloads go through the typed handle.

## Decision

- **A new Engine crate, `World_Reload`.** Every Engine crate needs config
  loading and Engine crates should not depend on each other, so the shared
  plumbing gets its own small, stable crate. It holds:
  - typed config assets: one loader per config type, parse and validate before
    applying, keep the last good value on any failure, mirror the value into a
    `Config<T>` resource, and record status per config in a `ConfigRegistry`
    (origin, applied and rejected counts, last error), with type-erased apply
    and atomic save by name for the developer panel;
  - the code-reload bridge: `CodeReloaded` (written for each `HotPatched` with
    the `hotpatch` feature), reloadable setup that runs at startup and again
    after each reload, and `ReloadScoped` entities that are despawned first.
- **The `ConfigAsset` trait lives in `World_Core`**, with no Bevy, so Foundation
  types such as the material table declare themselves tunable in their own
  crate. Each owning crate defines its type and registers it:
  materials in `World_Core` (registered by `World_Render`), physics tuning in
  `World_Physics`.
- **Two ways to run.** `cargo xtask dev` is `cargo run` with the `dev` feature:
  dynamic linking, asset, shader and config hot reload, developer panels.
  `cargo xtask hot` is `dx serve --hot-patch` with the `hotpatch` feature and no
  dynamic linking. On Windows, dx 0.7.10 needed four workarounds, all inside
  `tools/Xtask/src/hot`, each found by a failed link:
  1. dx cannot drive the `rust-lld` that `.cargo/config.toml` selects, and Git
     for Windows puts an unrelated GNU `link.exe` first on PATH.
  2. dx hands the linker an unquoted response file, so any argument with a space
     (`/LIBPATH:C:\Program Files\...`) breaks it (LNK1104). xtask names itself as
     the linker; as a shim it quotes dx's response files and forwards to the
     toolchain's `rust-lld -flavor link`. `lld-link` is required anyway, because
     dx's "fat binary" archive is GNU-format, which `link.exe` rejects (LNK4003).
  3. dx runs the fat-binary link itself, outside rustc, with no `LIB` paths
     (`ntdll.lib` not found), so dx starts inside the variables `vcvars64.bat` sets.
  4. dx reuses its cached fat archive without the standard-library rlibs that
     belong with it (undefined `__rustc::rust_panic`), so xtask deletes the
     cached archive before every start. It refuses to start when
  the installed `dx` does not match `subsecond` in `Cargo.lock`.
- **Developer panels in `World_Game/ui/dev`**, behind the `dev_tools` feature:
  F1 entity inspector, F2 config editor (edits the file text so comments
  survive; Apply is live, Save writes atomically and the reload confirms it;
  invalid text is never applied or written), F5 re-runs reloadable setup.
- **`rust-lld` is the default linker on Windows** in `.cargo/config.toml`.
- **The composite pass exists as version 0**, a sky gradient from a WESL shader
  that imports a shared module, so shader hot reload is real and tested before
  the voxel passes arrive.
- **Testing.** Config reload is tested deterministically through an in-memory
  asset source with a hand-driven watcher, and end to end through a real watched
  folder. Code reload is tested under shuffled schedules. Shader reload is
  tested on the GPU (software adapter) by editing files while rendering.
  `cargo xtask hot --smoke` proves the whole hot-patch chain by editing a probe
  in the running game. `World_Testkit_Bevy` gains asset sources for these tests
  and a wall-clock bound for waiting on background work, so its rule about the
  wall clock is restated: it bounds waiting, it never decides a result.

## Consequences

- Data tuning never needs a restart, and a bad edit can never take the game
  down or silently apply.
- Code hot-patching needs a separate, non-dynamic build of Bevy the first time
  it runs (about 20 minutes once). Measured on the RTX 4070 laptop: the game
  starts under dx in about 45 seconds, and an edit reaches it in about 26.
- The Windows workarounds are tied to dx 0.7.10. When dx is upgraded, try
  removing each one; `cargo xtask hot --smoke` shows whether the chain still works.
- Setup code that should follow code edits must be registered as reloadable and
  must tag what it spawns. Changing a struct's layout still needs a restart.
- The hot-patch smoke test runs weekly and on demand in CI, not on every push.
