# Audio

## Purpose

The audio domain turns gameplay events into sound. Its distinguishing idea is that sounds are driven by materials: what breaks, scrapes or lands determines what you hear, using the same material table that drives friction, strength and density.

## Scope

- Break and impact sounds selected by material pair and magnitude: the material of the voxels removed, the material struck, and the energy or volume involved.
- Mixing rules for destruction: grouping many simultaneous breaks into a few voiced sounds, limiting voice counts, ducking and priority.
- Spatialization: positional playback relative to the camera at the player's eyes, distance falloff, simple occlusion queries through the voxel tree.
- Footsteps and body sounds driven by the ground material under the player.
- Tool and ambient sounds, and the data that maps sounds to materials and events, loaded as hot-reloadable assets.

## Out of scope

- Detecting hits and breaks belongs in `interaction` and World_Physics; audio only listens to events.
- The material table itself, including break sound identifiers, belongs in World_Core.
- Music and menu click sounds triggered by interface actions are requested by `ui`.
- Visual feedback belongs in `effects`.

## Relationships

Consumes events from `interaction`, `tools`, `creatures` and `player`. Reads the material table from World_Core. Uses the audio backend provided through the engine's Bevy dependency.

## Rules

- Audio never changes game state.
- Event volume is bounded: a collapse generating thousands of breaks must collapse into a bounded number of voices.
- Sound selection depends on material identifiers and data, never on hard-coded object types.
- Systems stay in `Update` and avoid changing live component layouts.
