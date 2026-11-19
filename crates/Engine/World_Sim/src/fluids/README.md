# fluids

## Purpose

This folder is the home for liquid simulation: water, lava, and other flowing materials. It is the most expensive simulation in the crate, since at 1/64 m (1.56 cm) a single cubic metre holds 262,144 voxels. The design therefore assumes the simulation runs coarser than render resolution or within bounded regions.

## Scope

- Fluid state: per-cell volume or fill level on a simulation grid, with the grid coarser than the render voxels and mapped back onto them for display and collision.
- Flow rules: gravity, pressure equalization, and lateral spreading, using a cellular approach rather than a full Navier-Stokes solve.
- Bounded regions: simulating only where fluid is present or disturbed, with a hard cap on active cells, and settled pools going to sleep.
- Interaction with the voxel world: filling containers, flowing around and through broken terrain, eroding or being blocked by edits, and reacting when voxels are removed beneath it.
- Material interaction: water extinguishing fire, lava igniting neighbors, and density-based layering between fluids, all driven by the material table.
- Buoyancy and drag data for bodies in fluid, supplied to World_Physics as forces.
- Surface and effect data for the renderer, such as a fluid height field or surface voxels.

## Out of scope

- Granular materials: sand.
- Rigid-body forces beyond supplying buoyancy values: World_Physics.
- Rendering of water, reflections, and refraction: World_Render.
- Deciding sources and sinks: World_Game.

## Relationships

Depends on World_Core, World_Physics (buoyancy interface), and budget. Depended on by the crate's schedule. Interacts with fire and sand through voxel and material state only.

## Rules

- Volume is conserved, apart from explicit sources and sinks.
- Active cell count and per-frame work are capped by the shared budget.
- Fluid edits to the voxel tree go through the edit API.
- It can be disabled entirely without affecting other simulations.
