# Cargo Configuration

## Purpose

This folder holds workspace-wide Cargo configuration that applies to every build, run and test in the repository, no matter which crate or directory the command is started from. It exists so that environment and build settings every developer and every CI machine must share are written down once, in version control, instead of living in personal shell profiles.

## Scope

- Environment variables that Cargo sets for every compiled program and every `cargo run`, most importantly the one that points Bevy's asset loader at the shared top-level `assets/` folder instead of a folder inside the binary crate.
- Cargo aliases that give short names to long, frequently used commands, including the entry point that launches the `Xtask` automation runner.
- Target-specific build settings such as linker choice and linker flags, per operating system and CPU, when a platform needs them.
- Build-wide switches that are not tied to one crate, such as incremental or caching behavior and network settings for dependency fetching.

## Out of scope

- Dependency versions, features, profiles and lint levels: the workspace `Cargo.toml` at the repository root owns those.
- Per-crate settings: each crate's own manifest.
- Toolchain version and components: `rust-toolchain.toml` at the repository root.
- Personal preferences of one developer: use the user-level Cargo configuration instead, never this folder.
- Secrets, tokens and registry credentials of any kind.

## Relationships

Cargo reads this folder automatically for any command run inside the repository. `app` relies on it for asset discovery, `tools/Xtask` for its alias, and CI inherits the same settings so that a local build and a CI build behave identically.

## Rules

- Every entry is shared by everyone; nothing machine-specific or user-specific belongs here.
- Settings stay minimal and each one carries a comment explaining why it exists.
- A setting that only one crate needs goes into that crate's manifest, not here.
