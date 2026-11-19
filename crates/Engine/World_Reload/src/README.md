# World_Reload source

## Purpose

The source root of the reload crate. It exposes the crate's one plugin and prelude and composes the two module folders beside it: data reloading in `config` and code reloading in `code`.

## Scope

- The public plugin, which records where configs live on disk and installs the code-reload path.
- The prelude and the re-exports other crates use: registering and reading configs, reloadable setup, the reload messages.
- Module declarations for `config` and `code`.

## Out of scope

- Any behavior: loading and validating configs belongs in `config`, reacting to code reloads in `code`.
- Config types and files, which belong to their owning crates and `assets/config`.

## Relationships

Depends on the two module folders, on Bevy and on World_Core. The two modules do not depend on each other.

## Rules

- One public plugin and one prelude. Types are public only when another crate needs them.
- Systems are ordered with system sets so they behave the same under randomized schedules.
