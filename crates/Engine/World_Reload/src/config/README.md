# Config

## Purpose

This module turns tunable RON files into live, typed values. A config type declares itself with the core config trait; this module loads its file through the asset server, parses and validates it, mirrors the value into a resource, follows every save while the game runs, and keeps the last good value when a save is broken. It also keeps the status of every config so people can see what is live and why a change was rejected.

## Scope

- The asset type and asset loader for a config, chosen by type so every config can be a `.ron`.
- Parsing and validating text, with errors that carry a position or the list of every invalid value.
- The live-value resource, its safe defaults, and applying loaded or reloaded files in a set that runs before gameplay systems.
- The registry: asset path, origin of the live value (defaults, file or hand edit), applied and rejected counts, last error, and the asset root on disk.
- Type-erased operations by name: current value as text, apply text, save text atomically after validating it.

## Out of scope

- The config types, their fields, units and ranges: the owning crates.
- The panels that show and edit configs: World_Game's ui domain.
- File watching itself: Bevy's asset server.

## Relationships

Uses World_Core's config trait and Bevy's asset server. Used by every crate that registers a config, and by the developer panels through the registry.

## Rules

- A value is applied only after it parses and validates; anything else leaves the live value untouched.
- Every rejection is recorded and logged with its reason.
- Saving never writes text that fails validation, and never leaves a half-written file.
- Several reload events in one frame apply the newest value once.
