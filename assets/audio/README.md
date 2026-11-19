# Audio

## Purpose

This folder holds the game's sound assets. Because destruction is the core interaction, most of the audio is driven by materials: what a thing is made of decides what it sounds like when it breaks, lands, scrapes or is struck.

## Scope

- Material-keyed break sounds, played when voxels are removed or an island splits off.
- Material-keyed impact and contact sounds for debris, tools and the player's footsteps, with variants per material so repeated events do not sound identical.
- Ambience beds for biomes and interiors, and environmental one-shots.
- Tool and player action sounds (swing, hit, place, dig).
- UI feedback sounds that are not tied to a material.

Grouping follows the material table in `assets/config`: a sound is filed by the material key that triggers it, so adding a material means adding its sounds here under the same key.

## Out of scope

- Audio playback code, mixing, spatialization and material-to-sound lookup logic. Those live in `World_Game` (and `World_Sim` for simulation-driven events).
- The mapping from material to sound set, which is data and belongs in `assets/config`.
- Music or sounds for tooling and tests.
- Source project files for audio editors that are not needed at runtime.

## Relationships

- Loaded through the asset server by `World_Game`; triggered by events that `World_Physics` and `World_Sim` emit (contact, break, island split).
- Keys must match materials defined in `assets/config`.
- Bundled into every platform package by `packaging/`.

## Rules

- Files are stored through Git LFS.
- Use a single agreed sample rate and channel layout per category so the mixer needs no per-file handling; one-shot effects are mono so they can be positioned in 3D.
- Loudness is normalized within a category; relative levels are tuned in config, not by re-exporting audio.
- Each material has at least a fallback sound, so a new material is never silent.
- File names are lowercase with underscores and carry the material key as a prefix.
