# 0003: Platform, determinism and ownership

Status: accepted, 2026-11-19

## Context

Two reviews of the plan, the folder READMEs and the hot-reload branch found
questions that no code depends on yet but soon will. Each one gets harder to
change once saves, shaders or crate boundaries rely on the answer. They were
settled together, before testing on the 2019 MacBook Pro (the performance
floor) begins.

## Decision

### macOS

- **The Mac builds from source.** The 2019 MacBook Pro is tested by cloning the
  repo and running the same commands as Windows (`docs/devlog/2026-11-19-macbook-first-test.md`).
- **Code hot-patching must work on the Mac too.** `cargo xtask hot --smoke` is
  run there; whatever breaks is fixed, not declared unsupported.
- **The Mac gets a real `.app` now.** A `cargo xtask package` command builds a
  universal (Apple Silicon and Intel) `World.app` with `assets/` inside,
  ad-hoc signed, and CI uploads it. A release build never uses the `dev`
  feature, and the game finds its assets inside the bundle without
  `BEVY_ASSET_ROOT`. Developer ID signing and notarization wait until the game
  is shared publicly.
- **PR #1 merges after the Mac test passes**, so `main` never carries a known
  macOS problem. The weekly hot-patch smoke test starts with that merge.

### Determinism

- **World generation uses portable float math.** Floats are allowed only
  through the pure-Rust `libm` crate, never `f32::sin` and friends, whose
  results differ between platform math libraries. Generation never depends on
  `HashMap` iteration order or parallel reduction order. Every save stores the
  generator version, and CI hashes the same seed and chunks on Windows and
  macOS and requires identical results. Props that cross chunk borders are
  generated identically from either side, and level-of-detail data never feeds
  back into the authoritative voxels.
- **Physics has two modes.** The game runs Rapier in parallel for speed.
  Replay tests and a `--deterministic` flag run it single-threaded with
  enhanced determinism, so recorded replay hashes stay exact.
  `--mac-floor` must cap Rapier's thread pool as well once the parallel mode exists.

### Ownership

- **Player settings live in World_Game.** The settings types and the menus that
  edit them are World_Game's. The app finds the per-user settings file, loads
  and saves it, and applies the settings that need a restart (backend, tier,
  brick pool) at startup. Tunable game data stays in `assets/config/`; player
  settings never go there.
- **The collision grid size is a World_Core constant**, not configuration. It
  shapes collider and save layout and never changes at runtime.
- **Types shared between Engine crates are plain data in World_Core.** A dynamic
  object's voxel grid, a debris particle batch and similar hand-offs are
  Bevy-free types in World_Core. Anim and Physics produce them as components,
  Render consumes them, and no Engine crate depends on another for them.
- **The CPU 64-tree is shared through snapshots.** An edit produces a new tree
  version by path copying. The renderer and brick streaming read an immutable
  snapshot taken once per frame, with no locks; old versions are freed when no
  snapshot uses them. The CPU tree stays the single source of truth.

## Consequences

- The voxel core (the next to-do item) is designed for path-copying snapshots
  from the start.
- World_Gen gains the `libm` dependency and a cross-platform golden-hash test
  when it is written; the CI macOS job runs that test.
- `assets/config/README.md` no longer lists the collision grid size as tunable.
- New work this creates: `cargo xtask package` and the CI `.app`, a
  `--deterministic` physics flag with replay tests, the World_Game settings
  domain with per-user storage, and whatever the Mac test turns up.
