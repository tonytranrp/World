# colliders

## Purpose

This category folder turns voxel data into collision geometry. It owns the decision of what gets a collider, at what resolution, and when it is built, rebuilt, or dropped. It covers both halves of the problem: the static world and dynamic objects.

## Scope

- The collision grid: a 1/16 m grid derived from the 1/64 m render voxels by downsampling occupancy, so one collision voxel summarizes 4x4x4 render voxels. The ratio and size come from World_Core constants.
- The policy for downsampling (which render voxels make a collision voxel solid, and how partial occupancy is treated).
- Static world colliders built per chunk (about 4 m), restricted to regions near awake bodies and the player, and invalidated and rebuilt when voxels in a chunk change.
- Dynamic object colliders built over each object's own dense grid, plus mass, center of mass, and inertia computed from per-voxel material density.
- Shared types for collider requests and lifetimes used by both sub-areas.

## Out of scope

- The Rapier calls that create shapes: rapier.
- Damage and splitting of objects: destruction, which triggers rebuilds here.
- Character movement: character.
- Material definitions and constants: World_Core.

## Relationships

Depends on rapier and World_Core. Depended on by destruction (after edits and splits it requests rebuilds) and character (which queries static colliders). Its two subfolders, chunk and dynamic, share the grid and downsampling definitions at this level and do not depend on each other.

## Rules

- Collider work is bounded: rebuilds are queued and time-sliced, never done unbounded in one frame.
- Colliders exist only where something can touch them; far chunks have none.
- Collision occupancy is always derived from the authoritative CPU tree or object grid, never stored independently.
- Density and friction come from the material table only.
