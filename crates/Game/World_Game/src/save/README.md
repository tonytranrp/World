# Save

## Purpose

The save domain orchestrates saving and loading a play session. The base world is regenerated from the seed per chunk, so a save holds only what the player changed: the edit overlay on the generated base, plus player state. This folder decides when and what to save and drives the process; the file format itself lives in World_Core's serialization.

## Scope

- Triggers: manual saves, autosave intervals, save on quit, and loading at startup or from a menu.
- Collecting state to persist: the edit overlay (voxel changes from digging and destruction, including dynamic objects that were split off), the world seed and generation parameters version, player position and orientation, held tool and inventory, and scenario state.
- Loading: regenerating the base from the seed, applying the overlay, restoring the player, and rebuilding derived data such as colliders.
- Save slots, metadata for the load menu, and handling of missing or incompatible saves.
- Background execution: serializing off the main thread so a save does not hitch the frame.

## Out of scope

- The binary format, compression and versioning of voxel data belong in World_Core's serialization.
- Terrain generation belongs in World_Gen.
- Menus for choosing slots belong in `ui`.
- The settings file belongs in `app/`.

## Relationships

Depends on World_Core for serialization and World_Gen for regeneration. Reads edits recorded by `interaction`, player state from `player`, and notifies `ui` about progress and errors.

## Rules

- A save moved between Windows and macOS must reproduce the same world: it depends only on the seed, the overlay and deterministic generation.
- Saving never blocks gameplay and never leaves a partial file; write to a temporary location and replace on success.
- Only deltas are saved; never write the generated base.
- Loading validates versions and fails with a clear message rather than loading a damaged world.
- Systems stay in `Update` and avoid changing live component layouts.
