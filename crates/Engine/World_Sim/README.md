# World_Sim

## Purpose

World_Sim holds the extra voxel simulations that go beyond rigid bodies: structures that fail under load, sand that flows, fire that spreads and burns, and fluids. It exists as its own crate so these systems can grow, be switched on or off, and be tested without touching the rigid-body physics crate.

## Scope

- Structural stress: a support graph over voxels, with load propagation and collapse when strength is exceeded. It builds on island and anchor detection from World_Physics, grading support instead of only asking whether a path to bedrock exists.
- Sand and falling-material cellular automata: granular voxels that fall, slide, and pile according to local rules and angle of repose.
- Fire: ignition, spread to neighbors, burning, and conversion to ash or removal, driven by flammability and heat values from the World_Core material table.
- Fluids: deferred and the most expensive at 1/64 m, so the design assumes they run coarser or only in bounded regions.
- A shared per-frame budget: all simulations draw from one time-slice allowance, so none can stall a frame and they share cost fairly.
- Active-region tracking, so only voxels with something to do are visited.

## Out of scope

- Rigid bodies, colliders, island detection, damage, and the character controller: World_Physics.
- Voxel storage, edits, and material data: World_Core.
- Drawing fire, smoke, or fluid surfaces: World_Render.
- Gameplay rules about when to ignite or pour: World_Game.

## Relationships

Depends on World_Core and World_Physics. Depended on by World_Game. World_Render does not depend on it; renderer-visible changes arrive as voxel edits and effect data. Inside the crate, each simulation is an independent module that shares only the budget module.

## Rules

- All work is time-sliced under the shared budget; no simulation iterates unbounded over its region.
- Simulations edit the CPU 64-tree only through World_Core's edit API, then let World_Physics react by rebuilding colliders and detecting islands.
- Material behavior comes from the World_Core table, never local constants.
- Each simulation is switchable and tested against a simple dense reference.
