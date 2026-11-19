# App

## Purpose

This module builds and drives a headless Bevy App for tests. It turns "create an app, add the plugin under test, step N frames, then look at the world" into a few calls, so ECS tests are short and every one advances time the same way.

## Scope

- A builder that creates an App on MinimalPlugins, lets the test add the plugins, resources and systems it needs, and fixes the time step.
- Stepping exactly N frames, and stepping fixed-update ticks for 60 Hz scenarios.
- Running until a condition on the world holds, with a mandatory frame cap and a failure message that describes the world when the cap is reached.
- Inspecting the world: reading resources, querying components and counting entities, without each test writing its own boilerplate.

## Out of scope

- System-order randomization: schedule.
- GPU, windows and rendering: render.
- The behavior under test and the assertions about it: the calling test.
- Gameplay fixtures, which belong to the crates under test.

## Relationships

The base of the harness. Schedule and render both operate on the App built here. Used by ECS integration tests across the Engine crates and World_Game, and by plugin suites in the root `tests/` folder.

## Rules

- Time never comes from the wall clock. Each update advances by a fixed, explicit step.
- Every run-until has a bound and cannot hang.
- The builder adds nothing that opens a window or touches audio or input devices.
- Building an app is cheap enough to do per test. No app is shared between tests.
