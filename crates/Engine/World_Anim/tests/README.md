# World_Anim tests

## Purpose

This folder holds integration tests for the animation crate, exercising it through its public Plugin and prelude. Per-function unit tests live next to the code in the source tree.

## Scope

- Rig loading: valid and invalid RON rigs are accepted or rejected with the right errors, and hot reload updates a live skeleton while keeping persistent bone entities.
- Part attachment: a part follows its bone through animated motion, and the head part is hidden from the main view while remaining visible for shadow and reflection purposes.
- Import: .vox fixtures converted to grids compared against expected voxel counts, positions, and material mappings, including error cases for unmapped colors.
- IK: two-bone solutions checked against analytic expectations (the foot reaches reachable targets, limits are respected, unreachable targets fully extend the limb), and foot placement on synthetic stepped and sloped voxel ground built from a dense reference array.
- Procedural motion: gait phase, stride, and spring stability across timesteps and stiffnesses, checked for bounded energy.
- Ragdoll: one body per part and one joint per bone, mode switching preserves velocities, and a joint exceeding its strength releases the limb as an independent object.
- Headless App tests using `MinimalPlugins` with schedule randomization enabled, to catch system-ordering assumptions.

## Out of scope

- Physics correctness: World_Physics tests.
- Rendering of parts: World_Render golden-image tests.
- Timing and frame-cost measurement: handled by the workspace-level frame-time checks, not by tests.

## Relationships

Depends on the crate's public API, World_Physics, World_Core, and small checked-in rig and voxel fixtures. Nothing depends on this folder.

## Rules

- Tests are headless, deterministic, and use fixed seeds.
- Solvers are checked against closed-form or brute-force references, not their own output.
- Fixtures are minimal and hand-checkable.
