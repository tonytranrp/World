# World_Testkit_Bevy Source

## Purpose

This folder is the module tree of the World_Testkit_Bevy crate. It defines the crate's small public surface and arranges the Bevy-side test support into one deep module folder per concern.

## Scope

- The crate entry point, which exposes a prelude and, where the harness must register systems or resources in an App, one test-only plugin.
- One subfolder per concern: the headless App, schedule randomization, and offscreen rendering.
- Small shared types that several modules need, such as the error and report types used in failure messages.

## Out of scope

- Anything that does not need Bevy: World_Testkit.
- Tests of the harness: the sibling tests folder.
- Production plugins and the behavior they implement.
- Large grab-bag utility modules. If something is reusable, name the concept and give it a folder.

## Relationships

The App module is the base. Schedule configures an App, and render adds GPU and capture on top of one. No module imports one that depends on it. Profile and policy decisions come from World_Testkit and are never duplicated here.

## Rules

- Everything is test support and is never imported by shipping code.
- Public items are documented with the guarantee they give, not only their signature.
- No global mutable state. Seeds, adapters and resolutions are passed in, or read once from the environment at the edge.
- Module folders are snake_case and own their private helpers. Cross-module sharing goes through the owning module's public surface.
