# Policy

## Purpose

This module decides what a test does when a resource it needs is missing, most often a GPU adapter. On a developer machine a missing adapter should not block unrelated work. In CI it must never become a green run that tested nothing. The policy draws that line in one place.

## Scope

- Reading `WORLD_TEST_REQUIRE_GPU`. When it is `1`, a missing required resource is a hard failure; otherwise the test skips.
- The skip itself: a loud, greppable line beginning with `SKIPPED` that names the test and the missing resource, so skips are visible in every log.
- The decision as a pure function from (resource available, strictness) to run, skip or fail, together with the message to print. Environment access wraps this function and holds no logic.
- Any required resource, with GPU adapters as the main case. Tests that need one carry the `gpu_` name prefix.

## Out of scope

- Choosing or creating an adapter, software or real: World_Testkit_Bevy.
- Selecting which tests run: Xtask and the nextest configuration.
- Test scale: profile.

## Relationships

A leaf module. Called by GPU-dependent tests through World_Testkit_Bevy and the root `tests/` crate. CI sets the strict variable and Xtask passes it through unchanged.

## Rules

- A skip is never silent and never counted as a pass without its `SKIPPED` line.
- With strict mode on, there is no skip path at all.
- `1` means strict, unset or `0` means lenient, and any other value is rejected, so a typo cannot weaken the CI gate.
- The decision function has no side effects and is tested over every input combination.
