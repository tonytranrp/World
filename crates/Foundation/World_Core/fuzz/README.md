# World_Core Fuzzing

## Purpose

This folder holds the cargo-fuzz project for World_Core. It exists to prove that decoding untrusted bytes, such as a corrupted or hostile save, never panics, never loops forever and never allocates without bound, and that every tree that decodes encodes back to the same bytes.

## Scope

- Fuzz targets for the tree decoder and the encode/decode round trip.
- The seed generator that writes valid encodings as the starting corpus.
- Shared helpers that turn random input into a well-formed header and checksum, so mutations reach the body.

## Out of scope

- Ordinary unit, property and model tests: the crate's `src` and `tests` folders.
- Performance measurement: the benches folder.
- Corpora and crash inputs: generated locally or in CI, never committed.

## Relationships

This is its own Cargo workspace, excluded from the main one, because cargo-fuzz needs a nightly toolchain and sanitizer flags. It depends on World_Core through a path dependency and uses only its public API. `cargo xtask fuzz` runs it locally, and the weekly `fuzz` job in the quality workflow runs it in CI.

## Rules

- A crash input that is found becomes a regular test in World_Core before the fix lands.
- Targets use only the public API, so a fuzz build never needs crate internals opened up.
- Resource limits are part of each target: a decode that uses more memory than its input justifies is a failure.
