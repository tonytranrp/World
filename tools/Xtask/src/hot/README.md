# Hot

## Purpose

The commands that run the game with live reloading, and the automated proof that Rust hot-patching works on a machine. Developers start the game through here so the right features, linker and tools are always used together.

## Scope

- Running the game with dynamic linking, asset, shader and config hot reload and the developer panels.
- Running the game under the dx CLI with Rust hot-patching, with the features, linker and asset root that combination needs.
- Checking that the installed dx matches the hot-patch library version in the lock file, with the install command when it does not.
- The smoke test: start the game under dx, edit a probe in the source, wait for the patched value in the game's log, restore the file and stop the game.

## Out of scope

- The reload behavior itself: World_Reload and the crates that use it.
- Ordinary test runs: the `testing` module.

## Relationships

Runs cargo and dx. Edits only the probe file in the app, and always restores it.

## Rules

- Never combine dynamic linking with hot-patching.
- Never leave the probe file edited or the game running, whatever the outcome.
- Fail with the reason and the end of the log, never hang: every wait has a timeout.
