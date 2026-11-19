# Profile

## Purpose

This module turns the question "how hard should this test try?" into numbers. It reads the selected test profile and yields the scale every property test runs at, so a quick check on every push and an exhaustive run before a milestone differ by one environment variable and nothing else.

## Scope

- Reading `WORLD_TEST_PROFILE` (`fast` or `full`) and the proptest environment overrides.
- The scale of each profile: `fast` runs 256 proptest cases per test; `full` runs 1000 cases of 1000 edits each, the 1,000,000 random edits the engine plan requires of the voxel core.
- Producing a ready proptest configuration with the chosen case count, plus the edits-per-case figure for strategies that build edit sequences.
- Defaults and errors: an unset variable means `fast`; an unrecognized value is an error that names the accepted values.
- Interpretation as a pure function from strings to a profile, so it can be tested without changing the process environment.

## Out of scope

- What to do when hardware is missing: policy.
- Which tests a runner selects and how many threads it uses: Xtask and the nextest configuration. Runner profiles and test profiles are separate concepts.
- Generating random values: strategies.

## Relationships

A leaf module with no dependencies inside the crate. Read by strategies and directly by property tests in every crate that uses the kit. Xtask sets the variable; this module only reads it.

## Rules

- The same profile gives the same scale on every machine.
- Scale numbers exist here and nowhere else. Tests never hard-code case counts.
- An explicit proptest override is honored, and the effective scale is printed once, so a log always shows what ran.
- The environment is read at the edge; the interpretation is pure.
