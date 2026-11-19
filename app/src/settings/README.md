# Settings

## Purpose

This folder owns the persisted user preferences of the game: the settings file, its schema, defaults and the code that reads and writes it. It is the durable counterpart of the command line.

## Scope

- The settings schema: preferred graphics backend, output resolution, internal render scale, brick pool size cap, upscaling choice, field of view, input bindings, audio volumes, and developer toggles.
- Locating the file in the platform's per-user configuration directory on Windows and macOS.
- Loading with defaults for missing values, validation and clamping of out-of-range values, and tolerant handling of unknown keys so files survive version changes.
- Saving, written atomically so a crash cannot corrupt the file.
- Merging with command-line overrides and exposing the final resolved configuration.
- Marking which settings need a restart: the backend, the tier and the brick pool size are fixed when the device is created.

## Out of scope

- Saved games and the world edit overlay belong in World_Game's save domain and World_Core's serialization.
- Parsing flags belongs in `cli`.
- Settings screens and widgets belong in World_Game's ui domain; they request changes through events.
- Tunable gameplay and material values belong in hot-reloadable data assets, not user settings.

## Relationships

Consumes overrides from `cli` and feeds the resolved configuration to `startup`. The ui domain in World_Game reads and edits it through the exposed API.

## Rules

- Defaults are always safe: a missing or broken file never prevents launch.
- Only user preferences live here; never world state.
- Settings take effect live where possible and on restart where the GPU device is involved, and each setting declares which.
- The file format is stable and human-editable.
