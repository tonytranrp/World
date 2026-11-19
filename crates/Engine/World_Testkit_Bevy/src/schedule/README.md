# Schedule

## Purpose

This module exposes hidden system-ordering assumptions. Bevy runs systems in an order that is only partly specified, and a test can pass for months because the unspecified part happens to be stable. Randomizing that order with a seed turns such an assumption into a failure that can be reproduced.

## Scope

- Enabling system-order randomization on an App's schedules with a given seed.
- Running one test body across several seeds, each on a fresh App, and stopping at the first failure.
- Naming the failing seed in the failure message, so that one seed replays it.
- The seed list for a run: fixed defaults, with the count scaled by the active test profile.
- Wrapping Bevy's own randomization facility, so a change to its API is absorbed in this module alone.

## Out of scope

- Building the App: app.
- Fixing an ordering bug. The fix is explicit ordering on the systems, in the crate that owns them.
- Hashing and comparing replays across seeds: World_Testkit replay and the root suites. This module supplies only the seeds and the loop.

## Relationships

Operates on the App built by app and takes its scale from World_Testkit's profile. Used by ECS integration tests, and by replay suites that must produce identical hashes under any system order.

## Rules

- A failure always names its seed.
- Seeds are fixed or printed. Randomness never comes from the clock.
- Randomization is a test feature and is never enabled outside tests.
- A test that fails under one seed is not retried or ignored. The ordering is fixed in the system.
