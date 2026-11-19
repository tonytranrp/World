# World_Testkit Source

## Purpose

This folder is the module tree of the World_Testkit crate. It defines the crate's small public API and arranges the support code into one deep module folder per concern, so each piece of test infrastructure can be understood, tested and changed on its own.

## Scope

- The crate entry point, which re-exports a deliberately small API.
- One subfolder per concern: test profile, skip policy, the dense reference model, input strategies, hashing, replay records, golden comparison and ray-hit comparison.
- Small shared types that several modules need and that are too small for their own folder, such as the error and report types used in failure messages.

## Out of scope

- Bevy-aware helpers: World_Testkit_Bevy.
- Tests and benchmarks of the kit: the sibling tests and benches folders at the crate root.
- Production code of any kind. Nothing here implements the tree or any other structure under test.
- Large grab-bag utility modules. If something is reusable, name the concept and give it a folder.

## Relationships

Module dependencies flow one way. Profile, policy and hash sit at the bottom. Replay and golden build on hash, and strategies and hits build on the dense model. No module imports one that depends on it. Other crates consume only the API re-exported here and never reach into module internals.

## Rules

- Every module is test support only and is never imported by shipping code.
- Plain Rust: no Bevy types, no wgpu types, no global mutable state.
- Public items are documented with the guarantee they give, not only their signature.
- Reading the environment is confined to the modules that define a variable. The rest of the crate receives values as arguments.
- Module folders are snake_case and own their private helpers. Cross-module sharing goes through the owning module's public surface.
