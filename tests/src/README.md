# Tests Source

## Purpose

This folder is the library half of the root `tests` crate. It holds the small amount of shared code the cross-crate suites need, so that each suite contains only its scenario and its assertions.

## Scope

- Fixture path helpers: locating the golden, scenes and replays folders from the workspace root regardless of the directory the runner starts in, and composing a fixture's path from scene, adapter class and stage.
- Shared helpers for the suites: loading a named scene fixture, choosing adapter and skip behavior through the testkits, and setup that more than one suite repeats.
- Documented constants, such as an image tolerance, that several suites share.

## Out of scope

- Test cases: the suites folder.
- Fixture files: the `golden`, `scenes` and `replays` folders.
- Helpers used by one suite only: that suite's own file.
- General test support (the oracle, golden comparison, hashing, the App harness): the two testkit crates.

## Relationships

Depends on both testkit crates and on the engine crates the suites exercise. Used only by the suites folder. The locations it defines are the same ones the Xtask golden command writes to.

## Rules

- Paths derive from the workspace layout, never from the current directory or an absolute path.
- A helper moves here only when two suites need it. General-purpose support belongs in a testkit crate.
- No test cases live here.
- Nothing here is imported by shipping code.
