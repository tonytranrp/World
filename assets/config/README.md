# Config

## Purpose

This folder holds the game's tunable data, written as RON and loaded as hot-reloadable assets. Its job is to keep numbers out of Rust code so they can be changed by editing a file or by dragging a value in the bevy-inspector-egui panel, with no recompile.

## Scope

- The material table: per material color, roughness, emission, density, strength, friction, restitution and the keys for break and impact sounds. This one table drives rendering, physics and audio.
- Worldgen parameters: seeds, noise settings, biome and cave parameters, prop placement rules and material layering by depth and slope.
- Physics constants: fixed step settings, debris budgets such as the particle threshold and active-body cap, character controller step height. (The collision grid size is a World_Core constant, not configuration: decision 0003.)
- Render tier settings: internal resolution, brick pool size, update rates for the GI cache, and the presets for the tiers and the `--mac-floor` mode.
- Other gameplay tuning that benefits from live editing.

## Out of scope

- Rig and skeleton definitions. Those live in `assets/rigs`.
- Voxel model data. That lives in `assets/models`.
- Shader code. That lives in `assets/shaders`.
- Settings the player changes at runtime (backend, keybindings). Those are user settings stored outside the asset tree.
- Test-only parameter sets. Those belong in `tests/scenes` so the shipped config can change without breaking tests.

## Relationships

- Types that deserialize these files are defined in the owning crates: materials in `World_Core`, worldgen parameters in `World_Gen`, physics constants in `World_Physics`, tier settings in `World_Render`.
- `tools/Xtask` may read the material table to generate shader-side lookup constants.
- Consumed at runtime through the asset server from `app/`.

## Rules

- RON only, one concern per file, with comments explaining units and valid ranges.
- Every unit is explicit in field names or comments (metres, voxels, seconds); never mix them silently.
- Changing a value must be safe at runtime: the loading code validates and rejects bad data rather than applying it.
- Worldgen parameters affect determinism: changing them changes the generated world, so they are versioned with the save format by the code that reads them.
- Material ids are stable once used in saves; add materials, do not renumber them.
