# World_Testkit Tests

## Purpose

This folder tests the testkit itself. A wrong oracle makes every other test lie, so the kit's correctness is proven here through its public API, mainly by comparing it with a second, deliberately different and naive model.

## Scope

- Property tests showing that the dense model's edits and ray cast agree with a naive model written differently, such as a map of occupied voxels and a ray walk that tests every voxel in a bounding region.
- Strategy tests: generated values stay inside their bounds, include boundary and awkward cases, and shrink toward simpler inputs.
- Hash tests: known values stay fixed, the encoding does not depend on the platform, and equal state hashes equal.
- Replay tests: the text round trip, mismatch detection at the correct step, and a deliberately nondeterministic scenario being caught.
- Golden tests: tolerance boundaries, the diff image on failure, and no writes without the update switch.
- Hit tests: identical hits pass and every kind of single-field difference is flagged.
- Profile and policy tests over every input combination.

## Out of scope

- Tests of production crates: their own tests folders.
- Tests that need Bevy or a GPU: World_Testkit_Bevy and the root `tests/` crate.
- Throughput measurements: the benches folder.

## Relationships

Tests use only the public API of the crate and dev-only dependencies. The naive second model is written here, not in the crate, so the kit never contains two models of the same thing.

## Rules

- The naive model shares no code with the oracle.
- Known-answer tests pin hashes and formats to literal values.
- Tests never mutate the process environment. They call the pure interpretation and decision functions.
- Every test is reproducible: fixed or printed seeds, with shrunk failures kept as regression inputs.
