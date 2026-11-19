# App Source

## Purpose

The source tree of the executable. It holds the program entry point and the thin orchestration that joins the three startup concerns, each kept in its own subfolder: command-line parsing, settings and boot sequencing. The entry point itself only calls into those parts and then runs the Bevy application.

## Scope

- The entry point: parse arguments, load settings, run the startup sequence, run the app.
- The shared configuration type that merges settings-file values with command-line overrides, with a clear precedence order.
- Top-level error handling for fatal startup conditions, such as no usable graphics adapter, reported to the user in plain language.
- The `cli`, `settings` and `startup` subfolders.

## Out of scope

- Gameplay, rendering, physics or any engine logic; those live in the library crates.
- Reusable logic that could live in a library crate and be tested without a window.
- Build and packaging automation, which lives in `tools/`.

## Relationships

Uses the Engine and Game crates only through their plugins and preludes. The subfolders feed each other in sequence: `cli` produces overrides, `settings` produces persisted preferences, and `startup` consumes both to build the application. Nothing depends on this folder.

## Rules

- No game logic. A rule about how the game plays belongs in World_Game.
- Precedence of configuration is fixed and documented in one place: command line over environment variable over settings file over defaults.
- Keep the entry point short enough to read in one screen.
- Startup order is explicit and sequential, because the GPU device and tier must exist before render plugins register.
