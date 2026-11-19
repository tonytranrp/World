# World_Testkit

## Purpose

World_Testkit is the shared test-support library of the Foundation layer. It supplies the simple, obviously correct things every core crate's tests compare against: a dense reference voxel model, proptest strategies that feed it, stable hashing, replay records, golden-file comparison, an exact ray-hit comparator, and the policy that sets how hard a test tries and what happens when a GPU is missing. Writing these once means every layer is judged by the same oracle under the same rules. The crate has no Bevy dependency and never ships with the game.

## Scope

- Reference models: the dense voxel array with its edit and ray-cast operations, which is the oracle the sparse 64-tree in World_Core must match.
- Property-test strategies for positions, materials, edit sequences and rays, built on proptest so failures shrink to small cases.
- Hashing with xxh3-64 and a defined byte order, so recorded hashes mean the same on every machine and across library upgrades.
- Replay records: scenario name, step count and per-step hashes, stored as stable text, with the checks that prove a scenario deterministic and unchanged.
- Golden comparison for images (per-channel tolerance, diff image on failure) and for text and hash values, with an explicit update workflow.
- Ray-hit comparison: the hit record and an exact comparator between a CPU reference and a GPU tracer.
- Test profile and skip policy: the scale of the fast and full profiles, and the loud-skip or hard-fail decision when a required adapter is missing.

## Out of scope

- Anything that names Bevy, wgpu, a window or an ECS: World_Testkit_Bevy.
- The tests of other crates. Each crate owns its tests; this crate owns only the tests of the kit itself.
- Fixture files such as golden images, scenes and replays, which live under the root `tests/` folder.
- Running tests, regenerating goldens and judging benchmarks: Xtask and the nextest configuration.
- Production implementations. The kit holds the oracle, never the code being checked.

## Relationships

A Foundation crate. It depends at most on World_Core, for shared constants and plain types, plus third-party test libraries such as proptest, an xxh3 implementation and an image codec. World_Core, World_Gen and World_Cpu_Ref use it as a dev-dependency. World_Testkit_Bevy and the root `tests/` crate build on it.

## Rules

- Dev-dependency only. No shipping crate and no binary depends on it, and nothing here is compiled into a release build of the game.
- No Bevy, wgpu or physics dependency.
- Models are deliberately simpler than the code they check. Clarity beats speed, and nothing is shared with production code beyond constants and plain types.
- Everything is deterministic: fixed seeds, no wall clock, no dependence on thread count or platform.
- The environment is read only at the edges, in profile, policy and golden. The decision logic behind each is a pure function.
- Every helper is itself tested against a differently written naive model.
