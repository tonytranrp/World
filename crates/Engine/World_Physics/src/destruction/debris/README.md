# debris

## Purpose

This folder decides what each freed island becomes and keeps the number of live bodies under control. A sizable island becomes a rigid body. A tiny island becomes GPU particles. This is the safeguard against the physics stalls that unrestricted destruction would cause.

## Scope

- Island disposition: islands of about 64 voxels or more become rigid bodies; smaller ones become GPU particles. The threshold is a tunable value.
- Body spawning: extracting the island's voxels into a new dense grid, handing it to dynamic colliders for its collider and mass, and giving it the parent's velocity at that point plus the impulse of the break.
- Particle emission: producing particle spawn data (position, velocity, color from the material, lifetime) in a form World_Render consumes, with particles sleeping or settling after landing.
- The active-body cap of about 500: tracking the live count, preferring to merge, sleep, or demote the smallest and farthest bodies to particles when the cap is reached.
- Sleep management and lifetime: putting settled debris to sleep and retiring debris that is far from the player or is no longer worth simulating.
- Dust and spark effects tied to break events, as data only.

## Out of scope

- Rendering particles: World_Render.
- Connectivity analysis: islands. Support decisions: anchors.
- Collider and mass construction: colliders/dynamic.
- Break sound playback: audio in World_Game, using the sound data in the material table.

## Relationships

Depends on islands, anchors, colliders/dynamic, rapier, and World_Core. Depended on by World_Render (particle data) and World_Game.

## Rules

- The body cap is hard; the system degrades to particles rather than exceeding it.
- Sleeping is always on.
- Voxels are conserved: every voxel of an island ends up in a body or in particle data, never lost or duplicated.
- Spawning is bounded per frame.
