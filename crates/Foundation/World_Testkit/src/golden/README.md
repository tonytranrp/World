# Golden

## Purpose

This module compares test output with accepted reference output and manages deliberate updates of that reference. It covers images, where small tolerated differences exist, and text or hash values, where equality is exact.

## Scope

- Image comparison with a per-channel tolerance: the largest allowed difference on each channel. A size mismatch fails outright.
- On an image failure, a diff image written to the build output folder, never into the golden folder.
- Text and hash comparison: exact match, with a readable line diff on failure.
- The update workflow: references are rewritten only when `UPDATE_GOLDEN=1` is set or `cargo xtask golden --update` runs. An ordinary run never writes.
- Location rules: references resolve under the repository's golden and replay folders from a base path the caller supplies, and names carry scene, adapter class (software or gpu) and stage.
- Reading and writing lossless image files.

## Out of scope

- Producing the image: World_Testkit_Bevy and the renderer.
- The reference files: the `tests/golden`, `tests/scenes` and `tests/replays` folders.
- Choosing a tolerance for a scene: the test that calls this module.
- Regenerating many files and summarizing the change: the Xtask golden command.

## Relationships

Builds on hash for text and hash values and shares its update switch with replay. Used by render tests in Engine crates and by the root `tests/` crate. Xtask sets the update variable.

## Rules

- A missing reference is a failure that says how to create one, never an automatic pass.
- Update mode reports every file it writes, so a reviewer sees the full set.
- Tolerance is an explicit argument at each call site and defaults to exact. It is never loosened to make a failure pass without understanding it.
- Hashes and text are compared exactly. Tolerance applies to images only.
- Diff output is deterministic and small.
