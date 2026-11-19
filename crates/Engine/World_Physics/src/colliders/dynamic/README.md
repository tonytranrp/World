# dynamic

## Purpose

This folder gives moving objects their physical identity. Every dynamic object is a rigid body whose collider is a voxel collider over its own small dense grid. This folder builds that collider and derives the body's mass properties from the voxels.

## Scope

- Voxel colliders built over an object's dense grid at the 1/16 m collision resolution, downsampled from the object's render-resolution grid.
- Mass, center of mass, and inertia tensor computed by summing per-voxel material density across the object. These values are recomputed whenever the object loses or gains voxels.
- Per-object material aggregation for friction and restitution, derived from the materials of the voxels in contact.
- Updating a live collider incrementally when an object's grid is edited, and building the collider for a brand-new body created by a split.
- Re-centering: when the center of mass shifts, keeping the collider and the visual grid aligned in object space.

## Out of scope

- Static world colliders: chunk.
- Detecting that an object has split and extracting islands: destruction/islands.
- The Rapier calls themselves: rapier.
- The dense grid type and material table: World_Core.

## Relationships

Depends on the parent colliders folder, rapier, and World_Core. Depended on by destruction, which calls it whenever a split or damage event changes an object's voxels, and by World_Anim, whose rig parts are bodies with dynamic colliders. Sibling of chunk.

## Rules

- Mass is always the sum of voxel density times voxel volume, never an assigned number.
- Density, friction, and restitution come only from the World_Core material table.
- A collider must reflect the object's current grid; stale colliders are replaced, not patched around.
- Large per-object rebuilds are time-sliced like all other voxel-proportional work.
